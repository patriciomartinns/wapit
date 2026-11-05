import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(fileURLToPath(import.meta.url));
const origin = 'https://wapit.patriciomartins.com.br';

const locales = [
  {
    lang: 'en',
    source: 'content/docs.en.md',
    prefix: '',
    text: {
      title: 'Docs',
      description: 'Install, configure and use wapit, a local WhatsApp Cloud API for development.',
      skip: 'Skip to content',
      home: 'wapit home',
      main: 'Main',
      language: 'Language',
      onThisPage: 'On this page',
      llms: 'This page as plain text, for your coding agent.',
      copyPage: 'Copy as Markdown',
      viewPage: 'View as Markdown',
      copySection: 'Copy section as Markdown',
      copy: 'Copy',
      copyCode: 'Copy command',
      license: 'MIT license',
      notAffiliated: 'Not affiliated with Meta or WhatsApp.',
      madeBy: 'Made by Patrício Martins',
      version: 'Docs',
    },
  },
  {
    lang: 'pt-BR',
    source: 'content/docs.pt-br.md',
    prefix: '/pt-br',
    text: {
      title: 'Documentação',
      description: 'Instale, configure e use o wapit, uma WhatsApp Cloud API local para desenvolvimento.',
      skip: 'Pular para o conteúdo',
      home: 'Início do wapit',
      main: 'Principal',
      language: 'Idioma',
      onThisPage: 'Nesta página',
      llms: 'Esta página em texto puro, para o seu agente de código.',
      copyPage: 'Copiar como Markdown',
      viewPage: 'Ver como Markdown',
      copySection: 'Copiar seção como Markdown',
      copy: 'Copiar',
      copyCode: 'Copiar comando',
      license: 'Licença MIT',
      notAffiliated: 'Sem vínculo com a Meta ou com o WhatsApp.',
      madeBy: 'Feito por Patrício Martins',
      version: 'Documentação',
    },
  },
];

const logo = (size) =>
  `<svg width="${size}" height="${size}" viewBox="0 0 44 44" aria-hidden="true"><rect x="6" y="10" width="32" height="22" rx="3" fill="none" stroke="currentColor" stroke-width="3"/><path d="M6 13l16 10 16-10" fill="none" stroke="currentColor" stroke-width="3" stroke-linejoin="round"/><rect x="26" y="26" width="14" height="10" rx="2" fill="#ffd400"/></svg>`;

const markdownIcon =
  '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><rect x="1" y="3" width="14" height="10" rx="2" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M3.8 10.5v-5l2 2.4 2-2.4v5M10.8 5.5v5m0 0-1.6-1.7m1.6 1.7 1.6-1.7" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/></svg>';

const escape = (text) =>
  text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');

const slug = (text) =>
  text
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');

function inline(text) {
  return text
    .split(/(`[^`]+`)/)
    .map((part) => {
      if (part.startsWith('`') && part.endsWith('`') && part.length > 1) {
        return `<code>${escape(part.slice(1, -1))}</code>`;
      }
      return escape(part).replace(/https?:\/\/[^\s<]+[^\s<.,:;)]/g, (url) => {
        const external = !url.startsWith('http://127.0.0.1') && !url.startsWith('http://localhost');
        return external ? `<a href="${url}">${url.replace(/^https:\/\//, '')}</a>` : url;
      });
    })
    .join('');
}

function codeBlock(lines, id, text) {
  const body = lines
    .map((line) => (line && !line.startsWith(' ') ? `<span class="prompt">$ </span>${escape(line)}` : escape(line)))
    .join('\n');
  return `<div class="code"><pre id="${id}"><code>${body}</code></pre><button class="copy" type="button" data-copy="${id}" aria-label="${text.copyCode}"><span class="label">${text.copy}</span></button></div>`;
}

function table(rows) {
  const cells = (row) => row.replace(/^\||\|$/g, '').split('|').map((cell) => cell.trim());
  const [head, , ...body] = rows;
  return `<div class="table"><table><thead><tr>${cells(head).map((cell) => `<th scope="col">${inline(cell)}</th>`).join('')}</tr></thead><tbody>${body
    .map((row) => `<tr>${cells(row).map((cell) => `<td>${inline(cell)}</td>`).join('')}</tr>`)
    .join('')}</tbody></table></div>`;
}

function render(markdown, text) {
  const lines = markdown.split('\n');
  const html = [];
  const sections = [];
  let title = '';
  let lead = '';
  let codeCount = 0;

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];

    if (line.startsWith('# ')) {
      title = line.slice(2).trim();
    } else if (line.startsWith('> ')) {
      lead = line.slice(2).trim();
    } else if (line.startsWith('## ')) {
      const heading = line.slice(3).trim();
      const id = slug(heading);
      sections.push({ id, heading });
      html.push(
        `<div class="section-head${sections.length === 1 ? ' first' : ''}" id="${id}"><h2>${escape(heading)}</h2><button class="md-button" type="button" data-markdown-section="${escape(heading)}" aria-label="${text.copySection}: ${escape(heading)}">${markdownIcon}<span class="label">${text.copy}</span></button></div>`,
      );
    } else if (line.startsWith('```')) {
      const block = [];
      for (index += 1; index < lines.length && !lines[index].startsWith('```'); index += 1) block.push(lines[index]);
      codeCount += 1;
      html.push(codeBlock(block, `code-${codeCount}`, text));
    } else if (line.startsWith('|')) {
      const rows = [];
      for (; index < lines.length && lines[index].startsWith('|'); index += 1) rows.push(lines[index]);
      index -= 1;
      html.push(table(rows));
    } else if (line.startsWith('- ')) {
      const items = [];
      for (; index < lines.length && lines[index].startsWith('- '); index += 1) items.push(lines[index].slice(2));
      index -= 1;
      html.push(`<ul>${items.map((item) => `<li>${inline(item)}</li>`).join('')}</ul>`);
    } else if (line.trim()) {
      const paragraph = [line];
      for (; index + 1 < lines.length && lines[index + 1].trim() && !/^(#|>|```|\||- )/.test(lines[index + 1]); index += 1) {
        paragraph.push(lines[index + 1]);
      }
      html.push(`<p>${inline(paragraph.join(' '))}</p>`);
    }
  }

  return { title, lead, sections, body: html.join('\n') };
}

function page(locale, doc) {
  const { text, prefix, lang } = locale;
  const markdownUrl = `${prefix}/llms-full.txt`;
  const toc = doc.sections.map(({ id, heading }) => `<a href="#${id}">${escape(heading)}</a>`).join('\n        ');
  const link = (path, label, current) =>
    `<a href="${path}" hreflang="${label === 'EN' ? 'en' : 'pt-BR'}" lang="${label === 'EN' ? 'en' : 'pt-BR'}"${current ? ' aria-current="true"' : ''}>${label}</a>`;

  return `<!doctype html>
<html lang="${lang}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${text.title} · wapit</title>
  <meta name="description" content="${text.description}">
  <link rel="canonical" href="${origin}${prefix}/docs/">
  <link rel="alternate" hreflang="en" href="${origin}/docs/">
  <link rel="alternate" hreflang="pt-BR" href="${origin}/pt-br/docs/">
  <link rel="alternate" type="text/markdown" title="Markdown" href="${markdownUrl}">
  <link rel="icon" href="/favicon.svg" type="image/svg+xml">
  <link rel="preload" href="/fonts/geist-latin.woff2" as="font" type="font/woff2" crossorigin>
  <link rel="stylesheet" href="/site.css">
  <link rel="alternate" hreflang="x-default" href="${origin}/docs/">
  <meta property="og:type" content="article">
  <meta property="og:site_name" content="wapit">
  <meta property="og:locale" content="${lang === 'en' ? 'en_US' : 'pt_BR'}">
  <meta property="og:title" content="${text.title} · wapit">
  <meta property="og:description" content="${text.description}">
  <meta property="og:url" content="${origin}${prefix}/docs/">
  <meta property="og:image" content="${origin}/${lang === 'en' ? 'og.png' : 'og-pt-br.png'}">
  <meta property="og:image:width" content="1200">
  <meta property="og:image:height" content="630">
  <meta name="twitter:card" content="summary_large_image">
  <meta name="theme-color" content="#ffffff">
</head>
<body data-markdown="${markdownUrl}">
  <a class="skip" href="#main">${text.skip}</a>
  <header class="topbar">
    <a class="brand" href="${prefix}/" aria-label="${text.home}">
      ${logo(26)}
      wapit
    </a>
    <nav class="nav" aria-label="${text.main}">
      <a href="${prefix}/docs/" aria-current="page">Docs</a>
      <a class="mono" href="/llms.txt">llms.txt</a>
      <a class="with-icon" href="https://github.com/patriciomartinns/wapit"><svg class="icon" width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z"/></svg>GitHub</a>
      <span class="locale" role="group" aria-label="${text.language}">
        ${link('/docs/', 'EN', lang === 'en')}
        ${link('/pt-br/docs/', 'PT-BR', lang === 'pt-BR')}
      </span>
    </nav>
  </header>

  <div class="docs">
    <nav class="toc" aria-label="${text.onThisPage}">
      <p class="eyebrow">${text.onThisPage}</p>
        ${toc}
      <a class="llms" href="${markdownUrl}"><code>llms-full.txt</code><span>${text.llms}</span></a>
    </nav>
    <main class="content" id="main">
      <p class="eyebrow">${text.version} · v${locale.version}</p>
      <h1>${escape(doc.title)}</h1>
      <p class="lead">${inline(doc.lead)}</p>
      <div class="page-tools">
        <button class="md-button" type="button" data-markdown-page>${markdownIcon}<span class="label">${text.copyPage}</span></button>
        <a class="md-button" href="${markdownUrl}">${text.viewPage}</a>
      </div>
${doc.body}
    </main>
  </div>

  <footer class="footer">
    <div class="about">
      <a class="brand" href="${prefix}/">
        ${logo(22)}
        wapit
      </a>
      <p><a href="https://github.com/patriciomartinns/wapit/blob/main/LICENSE">${text.license}</a>. ${text.notAffiliated}</p>
    </div>
    <a class="author" href="https://github.com/patriciomartinns">
      <img src="https://github.com/patriciomartinns.png?size=72" width="36" height="36" alt="">
      <span><strong>${text.madeBy}</strong><span>@patriciomartinns</span></span>
    </a>
  </footer>
  <script src="/site.js" type="module"></script>
</body>
</html>
`;
}

const cargo = await readFile(join(root, '..', 'Cargo.toml'), 'utf8');
const version = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!version) throw new Error('No version in Cargo.toml');

for (const locale of locales) {
  const markdown = await readFile(join(root, locale.source), 'utf8');
  const doc = render(markdown, locale.text);
  const out = join(root, 'public', locale.prefix);
  await mkdir(join(out, 'docs'), { recursive: true });
  await writeFile(join(out, 'docs', 'index.html'), page({ ...locale, version }, doc));
  await writeFile(join(out, 'llms-full.txt'), markdown);
}

console.log(`Built the docs for v${version}`);
