const menus = [...document.querySelectorAll('.menu')];

function closeMenu(menu, focusButton) {
  const button = menu.querySelector('.menu-button');
  if (button.getAttribute('aria-expanded') !== 'true') return;
  button.setAttribute('aria-expanded', 'false');
  menu.querySelector('.menu-list').hidden = true;
  if (focusButton) button.focus();
}

function openMenu(menu) {
  for (const other of menus) if (other !== menu) closeMenu(other, false);
  menu.querySelector('.menu-button').setAttribute('aria-expanded', 'true');
  menu.querySelector('.menu-list').hidden = false;
  const items = [...menu.querySelectorAll('[role="menuitemradio"]')];
  (items.find((item) => item.getAttribute('aria-checked') === 'true') ?? items[0]).focus();
}

for (const menu of menus) {
  const button = menu.querySelector('.menu-button');
  const items = [...menu.querySelectorAll('[role="menuitemradio"]')];
  button.addEventListener('click', () => {
    if (button.getAttribute('aria-expanded') === 'true') closeMenu(menu, false);
    else openMenu(menu);
  });
  button.addEventListener('keydown', (event) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    openMenu(menu);
  });
  menu.querySelector('.menu-list').addEventListener('keydown', (event) => {
    const index = items.indexOf(document.activeElement);
    const next = { ArrowDown: index + 1, ArrowUp: index - 1, Home: 0, End: items.length - 1 }[event.key];
    if (event.key === 'Escape') {
      event.preventDefault();
      closeMenu(menu, true);
    } else if (event.key === 'Tab') {
      closeMenu(menu, false);
    } else if (next !== undefined) {
      event.preventDefault();
      items[(next + items.length) % items.length].focus();
    }
  });
}

document.addEventListener('click', (event) => {
  for (const menu of menus) if (!menu.contains(event.target)) closeMenu(menu, false);
});

const themeChoices = [...document.querySelectorAll('[data-theme-choice]')];
const themeButton = document.querySelector('[data-theme-button]');

function showTheme() {
  const current = document.documentElement.dataset.theme ?? 'system';
  for (const choice of themeChoices) {
    const checked = choice.dataset.themeChoice === current;
    choice.setAttribute('aria-checked', String(checked));
    if (checked && themeButton) {
      themeButton.replaceChildren(choice.querySelector('svg').cloneNode(true));
      themeButton.setAttribute('aria-label', `${themeButton.closest('.menu').querySelector('.menu-list').getAttribute('aria-label')}: ${choice.textContent.trim()}`);
    }
  }
}

for (const choice of themeChoices) {
  choice.addEventListener('click', (event) => {
    const value = choice.dataset.themeChoice;
    if (value === 'system') delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = value;
    try {
      if (value === 'system') localStorage.removeItem('wapit.theme');
      else localStorage.setItem('wapit.theme', value);
    } catch {}
    showTheme();
    closeMenu(choice.closest('.menu'), event.detail === 0);
  });
}

showTheme();

const copied ={ en: 'Copied', 'pt-BR': 'Copiado' }[document.documentElement.lang] ?? 'Copied';

function confirm(button) {
  const label = button.querySelector('.label') ?? button;
  const original = label.textContent;
  label.textContent = copied;
  button.dataset.state = 'copied';
  setTimeout(() => {
    label.textContent = original;
    delete button.dataset.state;
  }, 1600);
}

for (const button of document.querySelectorAll('[data-copy]')) {
  button.addEventListener('click', async () => {
    const source = document.getElementById(button.dataset.copy);
    try {
      await navigator.clipboard.writeText(source.innerText.replace(/^\$ /gm, ''));
      confirm(button);
    } catch {
      getSelection().selectAllChildren(source);
    }
  });
}

let markdown;

function loadMarkdown() {
  markdown ??= fetch(document.body.dataset.markdown).then((response) => {
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.text();
  });
  return markdown;
}

function section(text, title) {
  const parts = text.split(/^(?=## )/m);
  const match = parts.find((part) => part.slice(3, part.indexOf('\n')).trim() === title);
  return match?.trim() ?? text;
}

for (const button of document.querySelectorAll('[data-markdown-section], [data-markdown-page]')) {
  button.addEventListener('click', async () => {
    const title = button.dataset.markdownSection;
    const content = loadMarkdown().then((text) => (title ? `${section(text, title)}\n` : text));
    try {
      if (typeof ClipboardItem === 'function' && ClipboardItem.supports?.('text/plain') !== false) {
        const blob = content.then((text) => new Blob([text], { type: 'text/plain' }));
        await navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob })]);
      } else {
        await navigator.clipboard.writeText(await content);
      }
      confirm(button);
    } catch {
      window.open(document.body.dataset.markdown, '_blank', 'noopener');
    }
  });
}

if (document.body.dataset.markdown) loadMarkdown().catch(() => {});

const links = new Map([...document.querySelectorAll('.toc a[href^="#"]')].map((link) => [link.hash.slice(1), link]));

const current = document.querySelector('[data-toc-current]');
const mobileToc = document.querySelector('.toc-mobile');

for (const link of document.querySelectorAll('.toc-mobile a')) {
  link.addEventListener('click', () => { mobileToc.open = false; });
}

if (links.size > 0) {
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      for (const link of links.values()) link.classList.remove('active');
      const link = links.get(entry.target.id);
      link?.classList.add('active');
      if (link && current) current.textContent = link.textContent;
    }
  }, { rootMargin: '0px 0px -70% 0px' });

  for (const id of links.keys()) {
    const heading = document.getElementById(id);
    if (heading) observer.observe(heading);
  }
}
