const copied = { en: 'Copied', 'pt-BR': 'Copiado' }[document.documentElement.lang] ?? 'Copied';

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
