const tabs = [...document.querySelectorAll('.codes [role="tab"]')];

if (tabs.length > 0) {
  const panel = document.getElementById('error-panel');
  const caption = panel.querySelector('[data-error-caption]');
  const status = panel.querySelector('[data-error-status]');
  const body = panel.querySelector('[data-error-body]');
  const escape = (text) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');

  const select = async (tab, focus) => {
    const { errors } = await import('/errors.js');
    const error = errors[tab.dataset.code];
    if (!error) return;
    for (const other of tabs) {
      const selected = other === tab;
      other.setAttribute('aria-selected', String(selected));
      other.tabIndex = selected ? 0 : -1;
    }
    panel.setAttribute('aria-labelledby', tab.id);
    caption.textContent = error.caption;
    status.textContent = error.status;
    body.innerHTML = escape(JSON.stringify(error.body, null, 2)).replace(/^(\s*)("code": \d+,?)$/m, '$1<mark>$2</mark>');
    if (focus) tab.focus();
  };

  for (const tab of tabs) {
    tab.addEventListener('click', () => select(tab, false));
    tab.addEventListener('keydown', (event) => {
      const index = tabs.indexOf(tab);
      const next = { ArrowDown: index + 1, ArrowUp: index - 1, Home: 0, End: tabs.length - 1 }[event.key];
      if (next === undefined) return;
      event.preventDefault();
      select(tabs[(next + tabs.length) % tabs.length], true);
    });
  }
}

const themeButtons =document.querySelectorAll('[data-theme-choice]');

function showTheme() {
  const current = document.documentElement.dataset.theme ?? 'system';
  for (const button of themeButtons) button.setAttribute('aria-pressed', String(button.dataset.themeChoice === current));
}

for (const button of themeButtons) {
  button.addEventListener('click', () => {
    const choice = button.dataset.themeChoice;
    if (choice === 'system') delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = choice;
    try {
      if (choice === 'system') localStorage.removeItem('wapit.theme');
      else localStorage.setItem('wapit.theme', choice);
    } catch {}
    showTheme();
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

if (links.size > 0) {
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      for (const link of links.values()) link.classList.remove('active');
      links.get(entry.target.id)?.classList.add('active');
    }
  }, { rootMargin: '0px 0px -70% 0px' });

  for (const id of links.keys()) {
    const heading = document.getElementById(id);
    if (heading) observer.observe(heading);
  }
}
