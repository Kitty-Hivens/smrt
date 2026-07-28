<script lang="ts">
  import { t, i18n, LOCALES } from '../lib/i18n.svelte';
  import { theme, type ThemeChoice } from '../lib/theme.svelte';
  import Section from './ui/Section.svelte';

  // The panel's own preferences. Until this existed the only setting the product
  // had -- the locale -- was wedged into the top bar because there was nowhere
  // else to put it, and the theme could not exist at all.
  const THEMES: { value: ThemeChoice; label: () => string; hint: () => string }[] = [
    { value: 'system', label: () => t('set.themeSystem'), hint: () => t('set.themeSystemHint') },
    { value: 'dark', label: () => t('set.themeDark'), hint: () => t('set.themeDarkHint') },
    { value: 'light', label: () => t('set.themeLight'), hint: () => t('set.themeLightHint') },
  ];

  const LOCALE_LABEL: Record<string, string> = { ru: 'Русский', en: 'English' };
</script>

<div class="view">
  <Section title={t('set.appearance')}>
    <div class="choices" role="radiogroup" aria-label={t('set.theme')}>
      {#each THEMES as opt}
        <button
          class="choice"
          class:active={theme.choice === opt.value}
          role="radio"
          aria-checked={theme.choice === opt.value}
          onclick={() => theme.set(opt.value)}
        >
          <span class="swatch {opt.value}" aria-hidden="true"></span>
          <span class="ctext">
            <span class="clabel">{opt.label()}</span>
            <span class="chint">{opt.hint()}</span>
          </span>
        </button>
      {/each}
    </div>
  </Section>

  <Section title={t('set.language')}>
    <div class="choices" role="radiogroup" aria-label={t('set.language')}>
      {#each LOCALES as loc}
        <button
          class="choice"
          class:active={i18n.locale === loc}
          role="radio"
          aria-checked={i18n.locale === loc}
          onclick={() => i18n.set(loc)}
        >
          <span class="loccode mono" aria-hidden="true">{loc.toUpperCase()}</span>
          <span class="ctext"><span class="clabel">{LOCALE_LABEL[loc] ?? loc}</span></span>
        </button>
      {/each}
    </div>
  </Section>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .choices {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    text-align: left;
    padding: var(--space-3);
  }
  .choice.active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .ctext {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .clabel {
    font-size: var(--fs-md);
  }
  .chint {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--fg-dim);
  }
  /* the swatch shows the substrate itself rather than naming it: the field, a
     card on it, and the seam between them */
  .swatch {
    flex: none;
    width: 34px;
    height: 34px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--seam-bright);
    position: relative;
    overflow: hidden;
  }
  .swatch.dark {
    background: #000;
  }
  .swatch.light {
    background: #efeae1;
  }
  .swatch.system {
    background: linear-gradient(105deg, #000 0 50%, #efeae1 50% 100%);
  }
  .swatch::after {
    content: '';
    position: absolute;
    left: 6px;
    right: 6px;
    bottom: 6px;
    height: 10px;
    border-radius: 3px;
    background: var(--panel-3);
  }
  .loccode {
    flex: none;
    width: 34px;
    height: 34px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    border: 1px solid var(--seam-bright);
    font-size: var(--fs-xs);
    color: var(--fg-dim);
  }
</style>
