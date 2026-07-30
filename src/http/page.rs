//! Cursor paging for the listings that have no natural ceiling.
//!
//! Keyset rather than offset: a page is "everything after this row", so the
//! answer does not shift under a caller when rows are inserted while they walk,
//! and the database seeks to the cursor instead of counting past the rows it is
//! skipping. `LIMIT ... OFFSET n` gets slower the further in you read; this does
//! not.
//!
//! It is asked for rather than imposed. Without `limit` a listing answers whole,
//! the way it always has, so no existing caller is broken by pagination
//! arriving; with `limit` the answer is a page and the `Link` header names the
//! next one. The body shape is identical either way -- a client that pages and a
//! client that does not parse the same JSON, which is why the cursor rides in a
//! header instead of wrapping every listing in an envelope.

use axum::Json;
use axum::http::{HeaderValue, Uri, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;

/// Ceiling on one page, whatever was asked for. Large enough that a caller
/// paging in good faith never notices it, small enough that it stays a page.
const MAX_PAGE: usize = 500;

/// The paging half of a listing's query string: `?limit=<n>&after=<cursor>`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PageQuery {
    /// Absent -- the whole listing, as before paging existed.
    pub limit: Option<usize>,
    /// The cursor from the previous page's `Link`; opaque to the caller.
    pub after: Option<String>,
}

impl PageQuery {
    /// How many rows to read to answer this page, or `None` for all of them.
    /// One past the page so the caller can tell a full page from the last one
    /// without counting the whole table.
    pub fn probe(&self) -> Option<usize> {
        self.limit.map(|n| n.clamp(1, MAX_PAGE).saturating_add(1))
    }

    /// The page size actually in force.
    pub fn size(&self) -> Option<usize> {
        self.limit.map(|n| n.clamp(1, MAX_PAGE))
    }

    /// The cursor's parts, as the endpoint that minted it wrote them. A cursor
    /// that does not decode is treated as absent: it names a place in a listing,
    /// and the honest answer to a place that cannot be read is the start.
    pub fn cursor(&self) -> Option<Vec<String>> {
        decode(self.after.as_deref()?)
    }

    /// Trim `rows` to the page, and mint the cursor the next one starts after.
    /// `key` says how a row names itself as a cursor; `None` back means this was
    /// the last page. Separate from [`answer`](Self::answer) because a listing
    /// that answers inside an envelope still pages the rows in it.
    pub fn split<T, F>(&self, mut rows: Vec<T>, key: F) -> (Vec<T>, Option<String>)
    where
        F: Fn(&T) -> Vec<String>,
    {
        let Some(size) = self.size() else {
            return (rows, None);
        };
        // the probe read one past the page: more than fits means there is more
        let more = rows.len() > size;
        rows.truncate(size);
        let next = more
            .then(|| rows.last().map(|last| encode(&key(last))))
            .flatten();
        (rows, next)
    }

    /// Trim `rows` to the page and answer with a `Link` to the next one.
    pub fn answer<T, F>(&self, rows: Vec<T>, uri: &Uri, key: F) -> Response
    where
        T: Serialize,
        F: Fn(&T) -> Vec<String>,
    {
        let (rows, next) = self.split(rows, key);
        let mut resp = Json(rows).into_response();
        if let Some(value) = next_link(uri, next.as_deref()) {
            resp.headers_mut().insert(header::LINK, value);
        }
        resp
    }
}

/// The `Link` header naming the next page, or nothing when this was the last.
pub fn next_link(uri: &Uri, cursor: Option<&str>) -> Option<HeaderValue> {
    let cursor = cursor?;
    HeaderValue::from_str(&format!("<{}>; rel=\"next\"", link(uri, cursor))).ok()
}

/// A cursor is the sort key it names, in the order the listing sorts by. Encoded
/// rather than spelled out because a key can be a mod's name, which may hold
/// anything a name can hold; URL-safe so it survives a query string untouched.
fn encode(parts: &[String]) -> String {
    URL_SAFE_NO_PAD.encode(parts.join("\u{1f}"))
}

fn decode(cursor: &str) -> Option<Vec<String>> {
    let raw = URL_SAFE_NO_PAD.decode(cursor).ok()?;
    let text = String::from_utf8(raw).ok()?;
    Some(text.split('\u{1f}').map(str::to_string).collect())
}

/// The next page's address: this request's, with the cursor moved on. Every
/// other parameter is carried over verbatim -- a page of a filtered listing is
/// a page of that same filtered listing.
fn link(uri: &Uri, cursor: &str) -> String {
    let mut query: Vec<String> = uri
        .query()
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with("after="))
        .map(str::to_string)
        .collect();
    query.push(format!("after={cursor}"));
    format!("{}?{}", uri.path(), query.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn rows(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("row{i}")).collect()
    }

    fn paging(limit: Option<usize>, after: Option<&str>) -> PageQuery {
        PageQuery {
            limit,
            after: after.map(str::to_string),
        }
    }

    async fn body_of(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn next_of(resp: &Response) -> Option<String> {
        resp.headers()
            .get(header::LINK)
            .map(|v| v.to_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn a_listing_nobody_paged_answers_whole() {
        let page = paging(None, None);
        assert_eq!(page.probe(), None);
        let resp = page.answer(rows(3), &"/v1/x".parse().unwrap(), |r| vec![r.clone()]);
        assert!(next_of(&resp).is_none(), "nothing to page to");
        assert_eq!(body_of(resp).await, r#"["row0","row1","row2"]"#);
    }

    // The probe reads one past the page; that extra row is how the answer knows
    // there is a next page, and it must not be served as part of this one.
    #[tokio::test]
    async fn a_full_page_names_the_next_and_keeps_the_probe_to_itself() {
        let page = paging(Some(2), None);
        assert_eq!(page.probe(), Some(3));
        let resp = page.answer(rows(3), &"/v1/x".parse().unwrap(), |r| vec![r.clone()]);
        assert_eq!(body_of_next(&resp), Some(vec!["row1".to_string()]));
        assert_eq!(body_of(resp).await, r#"["row0","row1"]"#);
    }

    #[tokio::test]
    async fn the_last_page_names_nothing_after_it() {
        let page = paging(Some(5), None);
        let resp = page.answer(rows(3), &"/v1/x".parse().unwrap(), |r| vec![r.clone()]);
        assert!(next_of(&resp).is_none());
        assert_eq!(body_of(resp).await, r#"["row0","row1","row2"]"#);
    }

    /// The cursor the response points at, decoded back into its parts.
    fn body_of_next(resp: &Response) -> Option<Vec<String>> {
        let link = next_of(resp)?;
        let cursor = link
            .rsplit("after=")
            .next()?
            .trim_end_matches("\">; rel=\"next\"");
        let cursor = cursor.trim_end_matches(">; rel=\"next\"");
        decode(cursor)
    }

    // A page of a filtered listing is a page of that same filtered listing:
    // whatever narrowed it has to survive into the next address.
    #[test]
    fn the_next_address_keeps_every_other_parameter() {
        let uri: Uri = "/v1/registry/mods?q=create&loader=forge&after=stale"
            .parse()
            .unwrap();
        let next = link(&uri, "fresh");
        assert!(next.starts_with("/v1/registry/mods?"));
        assert!(next.contains("q=create"));
        assert!(next.contains("loader=forge"));
        assert!(next.contains("after=fresh"));
        assert!(
            !next.contains("stale"),
            "the old cursor is replaced, not kept"
        );
    }

    #[test]
    fn a_cursor_round_trips_whatever_a_key_holds() {
        let parts = vec!["a name with spaces & = ?".to_string(), "42".to_string()];
        assert_eq!(decode(&encode(&parts)), Some(parts));
    }

    // A cursor is a place in a listing. Handed a place that cannot be read, the
    // honest answer is the start of the listing rather than an error about a
    // string the caller never wrote by hand.
    #[test]
    fn an_unreadable_cursor_reads_as_no_cursor() {
        assert_eq!(paging(Some(10), Some("not base64 at all!!")).cursor(), None);
    }

    #[test]
    fn a_page_can_be_asked_for_but_not_made_unbounded() {
        assert_eq!(paging(Some(10_000), None).size(), Some(MAX_PAGE));
        assert_eq!(paging(Some(0), None).size(), Some(1));
    }
}
