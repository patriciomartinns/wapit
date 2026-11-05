const themeButtons = document.querySelectorAll('[data-theme-choice]');

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

const sections = document.querySelectorAll('.section, .content > :nth-child(n+5)');

const starSlots = document.querySelectorAll('[data-stars]');

async function stars() {
  const key = 'wapit.stars';
  try {
    const cached = JSON.parse(sessionStorage.getItem(key) ?? 'null');
    if (cached && Date.now() - cached.at < 3_600_000) return cached.count;
  } catch {}
  const response = await fetch('https://api.github.com/repos/patriciomartinns/wapit', {
    headers: { Accept: 'application/vnd.github+json' },
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const count = (await response.json()).stargazers_count;
  try { sessionStorage.setItem(key, JSON.stringify({ count, at: Date.now() })); } catch {}
  return count;
}

if (starSlots.length > 0) {
  stars()
    .then((count) => {
      if (!Number.isInteger(count) || count < 1) return;
      const label = new Intl.NumberFormat(document.documentElement.lang, { notation: 'compact' }).format(count);
      const title = document.documentElement.lang === 'pt-BR' ? `${count} estrelas no GitHub` : `${count} stars on GitHub`;
      for (const slot of starSlots) {
        slot.textContent = label;
        slot.title = title;
        slot.setAttribute('aria-label', title);
        slot.hidden = false;
      }
    })
    .catch(() => {});
}

const firstDelay =document.querySelector('.content') ? 200 : 320;

if ('IntersectionObserver' in window) {
  const reveal = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add('shown');
      reveal.unobserve(entry.target);
    }
  }, { rootMargin: '0px 0px -8% 0px' });

  const visible = [];
  for (const section of sections) {
    if (section.getBoundingClientRect().top < innerHeight) visible.push(section);
    else reveal.observe(section);
  }
  requestAnimationFrame(() => {
    visible.forEach((section, order) => {
      section.style.setProperty('--delay', `${firstDelay + Math.min(order, 8) * 60}ms`);
      section.classList.add('shown');
    });
  });
} else {
  for (const section of sections) section.classList.add('shown');
}

document.documentElement.classList.add('ready');

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
