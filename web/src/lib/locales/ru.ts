import type { Dict } from './en';

export const ru: Dict = {
  'app.checkingSession': 'Проверка сессии...',
  'common.loading': 'Загрузка...',

  'nav.overview': 'Обзор',
  'nav.packs': 'Паки',
  'nav.servers': 'Сервера',
  'nav.featured': 'Витрина',
  'nav.cache': 'Кэш',

  'shell.signOut': 'Выйти',
  'shell.refresh': 'Обновить',
  'shell.health': 'v{version} / схема {schema}',
  'shell.locale': 'Язык',

  'login.subtitle': 'Админка зеркала. Вставь токен, чтобы продолжить.',
  'login.submit': 'Войти',
  'login.checking': 'Проверка...',
  'login.rejected': 'Отклонено. Проверь токен.',
  'login.foot': 'панель управления зеркалом smrt',

  'dialog.confirmTitle': 'Подтверждение',
  'dialog.inputTitle': 'Ввод',
  'dialog.ok': 'ОК',
  'dialog.cancel': 'Отмена',
  'dialog.delete': 'Удалить',

  'packs.new': 'Новый пак',
  'packs.newPrompt': 'ID нового пака (буквы, цифры, - _ .):',
  'packs.col.pack': 'Пак',
  'packs.col.mc': 'MC',
  'packs.col.latest': 'Последняя',
  'packs.col.tags': 'Теги',
  'packs.col.flags': 'Флаги',
  'packs.unbuilt': '(не собран)',
  'packs.flag.featured': 'рекомендуемый',
  'packs.flag.authoring': 'редактируемый',
  'packs.empty': 'Паков пока нет. Создай или загрузи из SC-архива.',

  'overview.packs': 'Паки',
  'overview.packsSub': 'собрано {built} / не собрано {unbuilt}',
  'overview.servers': 'Сервера',
  'overview.cache': 'Jar в кэше',
  'overview.cacheSub': '{size} / {orphan} бесхозных',
  'overview.authoring': 'С авторским конфигом',
  'overview.featured': 'Рекомендуемые: паки / сервера',
  'overview.takedown': 'В takedown',
};
