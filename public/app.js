import { locale, plural, t, translatePage } from './i18n.js';

const $ = (selector) => document.querySelector(selector);

const view = {
    state: { apps: [], conversations: [], failure_reasons: [] },
    current: null,
    conversation: null,
    selectedId: null,
    attachment: null,
    typingTimer: null,
    scrollToEnd: false,
};


async function api(path, options = {}) {
    const response = await fetch(`/api${path}`, {
        ...options,
        headers: { 'Content-Type': 'application/json', ...(options.headers ?? {}) },
    });
    const json = await response.json().catch(() => ({}));

    if (!response.ok) {
        const translated = json.code ? t(`error.${json.code}`, { detail: json.detail ?? '' }) : null;
        const known = translated !== null && translated !== `error.${json.code}`;
        const error = new Error(known ? translated : json.error ?? t('error.status', { status: response.status }));
        error.status = response.status;
        throw error;
    }

    return json;
}

function toast(message, isError = false) {
    const element = $('#toast');
    element.classList.toggle('error', isError);

    if (!element.matches(':popover-open')) element.showPopover();

    element.textContent = '';
    requestAnimationFrame(() => { element.textContent = message; });
    clearTimeout(toast.timer);
    toast.timer = setTimeout(() => element.hidePopover(), isError ? 7000 : 3000);
}

function escapeHtml(text) {
    return String(text ?? '').replace(/[&<>"']/g, (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]));
}

function safeHref(url) {
    return /^https?:\/\//i.test(String(url ?? '')) ? url : '#';
}

function formatWhatsapp(text) {
    return escapeHtml(text)
        .replace(/```([\s\S]+?)```/g, '<code>$1</code>')
        .replace(/(^|[\s(])\*(?!\s)([^*\n]+?)\*(?=[\s).,!?:;]|$)/g, '$1<b>$2</b>')
        .replace(/(^|[\s(])_(?!\s)([^_\n]+?)_(?=[\s).,!?:;]|$)/g, '$1<i>$2</i>')
        .replace(/(^|[\s(])~(?!\s)([^~\n]+?)~(?=[\s).,!?:;]|$)/g, '$1<s>$2</s>')
        .replace(/https?:\/\/(?:(?!&(?:quot|#39|lt|gt|amp);)[^\s<])+/g, (match) => {
            const rawTrail = /[.,!?:;)\uE001]*$/.exec(match)[0];
            const shown = match.slice(0, match.length - rawTrail.length);
            const href = shown.replace(/[\uE000\uE001]/g, '');
            let balance = 0;
            let lowest = 0;

            for (const char of shown) {
                if (char === '\uE000') balance += 1;
                if (char === '\uE001') balance -= 1;
                lowest = Math.min(lowest, balance);
            }

            const before = -lowest;
            const openAtEnd = before + balance;
            const closedByTrail = Math.min(openAtEnd, (rawTrail.match(/\uE001/g) ?? []).length);
            let trail = rawTrail;

            for (let index = 0; index < closedByTrail; index += 1) trail = trail.replace('\uE001', '');

            return `${'\uE001'.repeat(before)}<a href="${href}" target="_blank" rel="noreferrer">${'\uE000'.repeat(before)}${shown}${'\uE001'.repeat(openAtEnd)}</a>${'\uE000'.repeat(openAtEnd - closedByTrail)}${trail}`;
        });
}

function plain(text) {
    return String(text ?? '')
        .replace(/```/g, '')
        .replace(/(^|[\s(])[*_~]([^*_~\n]+?)[*_~](?=[\s).,!?:;]|$)/g, '$1$2')
        .replace(/\s+/g, ' ')
        .trim();
}

function time(ms) {
    return new Date(ms).toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' });
}

function sameDay(first, second) {
    return first.toDateString() === second.toDateString();
}

function dayOffset(ms) {
    const date = new Date(ms);
    const today = new Date();

    for (const offset of [-1, 0, 1]) {
        const candidate = new Date(today);
        candidate.setDate(today.getDate() + offset);

        if (sameDay(date, candidate)) return offset;
    }

    return null;
}

function dayLabel(ms) {
    const offset = dayOffset(ms);

    if (offset === 0) return t('day.today');
    if (offset === -1) return t('day.yesterday');

    return new Date(ms).toLocaleDateString(locale, { day: 'numeric', month: 'long' });
}

function shortWhen(ms) {
    const offset = dayOffset(ms);

    if (offset === 0) return time(ms);
    if (offset === -1) return t('day.yesterdayShort');

    return new Date(ms).toLocaleDateString(locale, { day: '2-digit', month: '2-digit' });
}

function formatContact(contact) {
    const match = /^55(\d{2})(\d{4,5})(\d{4})$/.exec(contact);

    if (match) return `+55 ${match[1]} ${match[2]}-${match[3]}`;

    return /^\d+$/.test(contact) ? `+${contact}` : contact;
}

function appOf(appId) {
    return view.state.apps.find((app) => app.id === appId);
}

function appName(appId) {
    return appOf(appId)?.name ?? appId;
}

function renderSidebar() {
    const container = $('#apps');
    container.innerHTML = '';

    for (const app of view.state.apps) {
        const section = document.createElement('section');
        section.className = 'app';
        const conversations = view.state.conversations.filter((conversation) => conversation.app_id === app.id);

        section.innerHTML = `
            <div class="app-head">
                <div>
                    <h2>${escapeHtml(app.name)}</h2>
                    <p class="mono">${escapeHtml(app.phone_number_id)}</p>
                    ${app.templates === 0 ? `<p class="warn">${t('app.noTemplates')}</p>` : ''}
                </div>
                <div class="app-actions">
                    <button type="button" class="link-button" data-new="${escapeHtml(app.id)}">${t('app.new')}</button>
                    ${app.can_verify ? `<button type="button" class="link-button" data-verify="${escapeHtml(app.id)}">${t('app.verify')}</button>` : ''}
                </div>
            </div>`;

        if (conversations.length === 0) {
            section.insertAdjacentHTML('beforeend', `<p class="app-empty">${escapeHtml(t('app.empty', { app: app.name }))}</p>`);
        }

        for (const conversation of conversations) {
            const row = document.createElement('button');
            row.type = 'button';
            row.className = 'row';
            row.setAttribute('aria-current', String(view.current?.app === app.id && view.current?.contact === conversation.contact));
            const last = conversation.last;
            const preview = last === null ? '' : plain(last.body || `[${last.kind}]`);
            const flag = conversation.flagged > 0 ? `<span class="flag">${plural('row.flagged', conversation.flagged)}</span>` : '';

            row.innerHTML = `
                <span class="row-top"><strong>${escapeHtml(conversation.name || formatContact(conversation.contact))}</strong><time>${shortWhen(conversation.last_at)}</time></span>
                <span class="preview">${flag}${escapeHtml(preview)}</span>`;
            row.addEventListener('click', () => openConversation(app.id, conversation.contact));
            section.append(row);
        }

        container.append(section);
    }

    container.querySelectorAll('[data-new]').forEach((button) => button.addEventListener('click', () => openNewConversation(button.dataset.new)));
    container.querySelectorAll('[data-verify]').forEach((button) => button.addEventListener('click', () => verifyWebhook(button.dataset.verify)));
}

function renderBanner() {
    const banner = $('#config-banner');
    const text = view.state.config_error ? t('banner.configError', { error: view.state.config_error }) : view.state.using_default_app ? t('banner.defaultApp') : '';

    banner.textContent = text;
    banner.hidden = text === '';
}

function statusLabel(message) {
    if (message.direction === 'out') return t(`status.out.${message.status}`);

    return message.status.startsWith('read') ? t('status.in.read') : t('status.in.received');
}

function inboundLabel(message) {
    const webhook = message.webhooks.at(-1);

    if (message.status.startsWith('read')) return t('status.in.read');
    if (webhook === undefined) return t('status.in.sending');
    if (webhook.status_code === null || webhook.status_code >= 300) return t('status.in.undelivered');

    return t('status.in.received');
}

function metaHtml(message, extraClass = '') {
    const label = message.direction === 'in' ? inboundLabel(message) : message.status === 'accepted' ? '' : statusLabel(message);
    const text = label === '' ? time(message.created_at) : `${time(message.created_at)}  ${label}`;

    return `<button type="button" class="meta ${extraClass}" data-details data-key="details:${escapeHtml(message.id)}" aria-label="${escapeHtml(t('meta.label', { time: time(message.created_at) }))}${label ? `, ${escapeHtml(label.toLowerCase())}` : ''}">${escapeHtml(text)}</button>`;
}

function mediaHtml(rendered) {
    const media = rendered.media;

    if (!media) return '';

    const src = media.id && !media.link ? `/media/${encodeURIComponent(media.id)}` : safeHref(media.link);
    const isImage = ['image', 'sticker'].includes(rendered.kind) || (rendered.kind === 'template' && media.mime_type === 'image');

    if (isImage && src !== '#') {
        return `<img src="${escapeHtml(src)}" alt="">`;
    }

    return `<div class="doc"><svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M4 1.5h5l3 3v10H4z" stroke="currentColor" stroke-width="1.3"/></svg><span>${escapeHtml(media.filename ?? media.mime_type ?? rendered.kind)}</span></div>`;
}

function bodyHtml(rendered) {
    if (Array.isArray(rendered.body_parts) && rendered.body_parts.length > 0) {
        const marked = rendered.body_parts.map((part) => (part.variable ? `\uE000${part.text.replace(/[\uE000\uE001]/g, '')}\uE001` : part.text)).join('');

        return formatWhatsapp(marked).replace(/\uE000/g, '<span class="var">').replace(/\uE001/g, '</span>');
    }

    return formatWhatsapp(rendered.body ?? rendered.caption ?? '');
}

function templateOrigin(rendered) {
    const variables = (rendered.body_parts ?? []).filter((part) => part.variable).length;
    const details = [rendered.template.language];

    if (variables > 0) details.push(plural('origin.variables', variables));

    return `<p class="origin"><span class="name">${escapeHtml(rendered.template.name)}</span><span>${escapeHtml(details.join(', '))}</span></p>`;
}

function appSlip(message) {
    const rendered = message.rendered;
    let html = '';

    html += mediaHtml(rendered);

    if (rendered.header) html += `<p class="header">${formatWhatsapp(rendered.header)}</p>`;

    if (rendered.body || rendered.caption) html += `<p class="body">${bodyHtml(rendered)}</p>`;

    if (rendered.footer) html += `<p class="footer">${formatWhatsapp(rendered.footer)}</p>`;

    return `${rendered.template ? templateOrigin(rendered) : ''}<div class="slip">${html}${metaHtml(message)}${buttonsHtml(message)}</div>`;
}

function explainRejection(message) {
    const app = appName(message.app_id);
    const template = message.rendered.template?.name ?? t('reject.templateFallback');
    const mismatch = message.error?.mismatch;

    if (mismatch) {
        const button = /^button at index (\d+)/.exec(mismatch.location);
        const where = mismatch.location === 'body' ? t('reject.where.body') : mismatch.location === 'header' ? t('reject.where.header') : t('reject.where.button', { number: Number(button?.[1] ?? 0) + 1 });

        return t('reject.mismatch', { app, sent: mismatch.sent, variables: plural('reject.variables', mismatch.sent), where, template, expected: mismatch.expected });
    }

    const code = message.error?.code;
    const known = [132001, 132015, 132016, 132018, 131008, 131009, 132012, 132005];

    return known.includes(code) ? t(`reject.${code}`, { app, template }) : t('reject.generic', { app });
}

function rejectedCard(message) {
    return `
        <div class="rejected">
            <p class="title"><strong>${escapeHtml(t('reject.title', { code: message.error?.code ?? '' }))}</strong><button type="button" class="meta" data-details data-key="details:${escapeHtml(message.id)}" aria-label="${escapeHtml(t('reject.label', { time: time(message.created_at) }))}">${time(message.created_at)}</button></p>
            <p>${escapeHtml(explainRejection(message))} ${escapeHtml(t('reject.outcome'))}</p>
            <p class="details">${escapeHtml(message.error?.details ?? message.error?.message ?? '')}</p>
        </div>`;
}

function userSlip(message) {
    const rendered = message.rendered;

    if (rendered.kind === 'button_reply') {
        return `<div class="slip tap"><span class="tap-label">${t('tap.label')}</span><span class="tap-text">${escapeHtml(rendered.body)}</span>${metaHtml(message)}</div>`;
    }

    const text = rendered.body ?? rendered.caption ?? '';
    const media = mediaHtml(rendered);
    const short = media === '' && !text.includes('\n') && text.length <= 40;

    return `<div class="slip ${short ? 'inline' : ''}">${media}${text ? `<p class="body">${formatWhatsapp(text)}</p>` : ''}${metaHtml(message)}</div>`;
}

const BUTTON_ICONS = {
    url: '<path d="M9 3.5h3.5V7M12.5 3.5 7 9M11 9.5v3H3.5V5h3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
    phone_number: '<path d="M5.2 2.5 6.6 5 5.4 6.3a7 7 0 0 0 4.3 4.3L11 9.4l2.5 1.4-.6 2.2c-5.4.3-10.2-4.5-9.9-9.9z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>',
    copy_code: '<rect x="5.5" y="5.5" width="7" height="7" rx="1" stroke="currentColor" stroke-width="1.4"/><path d="M3.5 10.5v-7h7" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
    reply: '<path d="M6.5 4 3 7.5 6.5 11M3 7.5h6a4 4 0 0 1 4 4v1" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>',
    list_reply: '<path d="M3 4.5h10M3 8h10M3 11.5h10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
};

function buttonIcon(type) {
    const icon = BUTTON_ICONS[type === 'quick_reply' ? 'reply' : type];

    return icon ? `<svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">${icon}</svg>` : '';
}

function buttonTarget(button) {
    if (button.type === 'url' || button.type === 'phone_number') return button.url ?? '';
    if (button.type === 'quick_reply') return button.payload ?? '';

    return button.id ?? button.url ?? '';
}

function buttonsHtml(message) {
    const buttons = message.rendered.buttons ?? [];

    if (message.direction !== 'out' || buttons.length === 0) return '';

    const tappable = (button) => ['reply', 'list_reply', 'quick_reply'].includes(button.type);

    const rows = buttons.map((button, index) => {
        const label = `${buttonIcon(button.type)}<span>${escapeHtml(button.text)}</span>`;
        const target = buttonTarget(button);
        const title = target === '' ? '' : ` title="${escapeHtml(target)}"`;

        if (button.type === 'url') {
            return `<a class="action" href="${escapeHtml(safeHref(button.url))}" target="_blank" rel="noreferrer"${title}>${label}</a>`;
        }

        if (button.type === 'phone_number' && button.url) {
            return `<a class="action" href="tel:${escapeHtml(String(button.url).replace(/[^\d+]/g, ''))}"${title}>${label}</a>`;
        }

        return `<button type="button" class="action" data-tap="${index}" data-key="tap:${escapeHtml(message.id)}:${index}"${title} ${tappable(button) ? '' : 'disabled'}>${label}</button>`;
    });

    return `<div class="actions">${rows.join('')}</div>`;
}

function buttonsDetail(message) {
    const buttons = message.rendered.buttons ?? [];

    if (message.direction !== 'out' || buttons.length === 0) return '';

    const rows = buttons.map((button) => {
        const target = buttonTarget(button);

        return `<li><span class="what">${escapeHtml(button.text)}</span><span class="kind">${escapeHtml(t(`button.kind.${button.type}`))}</span>${target ? `<span class="target mono">${escapeHtml(target)}</span>` : ''}</li>`;
    }).join('');

    return `<section class="section"><h3>${t('inspector.buttons')}</h3><ul class="buttons-log">${rows}</ul></section>`;
}

function statusActions(message) {
    if (message.direction !== 'out' || message.status === 'rejected' || message.id !== view.selectedId) return '';

    const reaction = message.type === 'reaction';
    const reasons = view.state.failure_reasons.map((reason) => `<option value="${reason.code}">${escapeHtml(t('simulate.failureOption', { code: reason.code, title: reason.title.slice(0, 48) }))}</option>`).join('');

    return `
        <div class="status-actions">
            <span class="label">${t('simulate.label')}</span>
            ${reaction ? '' : `<button type="button" data-status="delivered" data-key="status:delivered">${t('simulate.delivered')}</button>
            <button type="button" data-status="read" data-key="status:read">${t('simulate.read')}</button>`}
            <select data-failure data-key="status:failed" aria-label="${escapeHtml(t('simulate.failureLabel'))}"><option value="">${t('simulate.failure')}</option>${reasons}</select>
            <button type="button" class="danger" data-send-failure data-key="status:send-failure" disabled>${t('simulate.sendFailure')}</button>
        </div>`;
}

function deliveryHtml(message) {
    const webhook = message.webhooks.at(-1);

    if (!webhook) return '';

    const app = appName(message.app_id);

    if (webhook.status_code === null) {
        return `<p class="delivery failed">${escapeHtml(t('delivery.none', { error: webhook.error }))}</p>`;
    }

    const ok = webhook.status_code < 300;
    const text = t(ok ? 'delivery.ok' : 'delivery.refused', { app, status: webhook.status_code, ms: webhook.duration_ms });

    return `<p class="delivery ${ok ? '' : 'failed'}">${escapeHtml(text)}</p>`;
}

function windowLabel(conversation) {
    const element = $('#chat-window');
    const open = conversation.window_open_until !== null && conversation.window_open_until > Date.now();
    const until = conversation.window_open_until;

    element.classList.toggle('closed', !open);

    if (open) {
        element.querySelector('strong').textContent = t('window.open');
        element.querySelector('span').textContent = t('window.until', { day: dayOffset(until) === 0 ? t('day.todayLower') : t('day.tomorrow'), time: time(until) });
    } else {
        element.querySelector('strong').textContent = t('window.closed');
        element.querySelector('span').textContent = t('window.closedHint');
    }
}

function renderConversation() {
    const focusKey = document.activeElement?.dataset?.key;
    const focusInList = focusKey !== undefined && document.activeElement.closest('.messages, .inspector') !== null;
    const focusMessage = focusInList ? document.activeElement.closest('.message')?.dataset.id : undefined;

    paintConversation();

    if (focusInList) {
        const scope = focusMessage ? document.querySelector(`.message[data-id="${CSS.escape(focusMessage)}"]`) : document;
        (scope?.querySelector(`[data-key="${CSS.escape(focusKey)}"]`) ?? document.querySelector(`[data-key="${CSS.escape(focusKey)}"]`))?.focus();
    }
}

function paintConversation() {
    const conversation = view.conversation;
    const list = $('#messages');
    const atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 80;

    $('#empty-chat').hidden = conversation !== null;
    $('#chat-head').hidden = conversation === null;
    list.hidden = conversation === null;
    $('#composer').hidden = conversation === null;

    if (conversation === null) {
        const any = view.state.conversations.length > 0;
        $('#empty-title').textContent = t(any ? 'empty.titleAny' : 'empty.title');
        $('#empty-text').textContent = t(any ? 'empty.textAny' : 'empty.text');
        $('#empty-env').hidden = any;
        $('#empty-start').textContent = t(any ? 'empty.startAny' : 'empty.start');
        renderInspector();

        return;
    }

    const app = appName(conversation.app_id);
    const who = conversation.name || formatContact(conversation.contact);
    $('#chat-title').textContent = who;
    $('#chat-meta').textContent = conversation.name ? t('chat.metaNamed', { contact: formatContact(conversation.contact), app }) : t('chat.meta', { app });
    $('#composer-hint').textContent = t('composer.hint', { who, app });
    windowLabel(conversation);

    list.innerHTML = '';
    let lastDay = '';

    for (const message of conversation.messages) {
        const label = dayLabel(message.created_at);

        if (label !== lastDay) {
            list.insertAdjacentHTML('beforeend', `<p class="day">${label}</p>`);
            lastDay = label;
        }

        const element = document.createElement('div');
        element.className = `message ${message.direction === 'out' ? 'from-app' : 'from-user'} ${message.id === view.selectedId ? 'selected' : ''}`;
        element.dataset.id = message.id;

        if (message.status === 'rejected') {
            element.innerHTML = rejectedCard(message);
        } else if (message.direction === 'out') {
            element.innerHTML = `
                ${appSlip(message)}
                ${message.issues.map((issue) => `<p class="notice">${escapeHtml(issue.code === 'legacy' ? issue.text : t(`issue.${issue.code}`))}</p>`).join('')}
                ${message.status === 'failed' ? `<p class="notice error">${escapeHtml(t('notice.failed', { title: message.statuses.filter((status) => status.status === 'failed').at(-1)?.error?.title ?? '' }))}</p>` : ''}
                ${statusActions(message)}`;
        } else {
            element.innerHTML = `${userSlip(message)}${deliveryHtml(message)}`;
        }

        element.querySelector('.slip, .rejected').addEventListener('click', (event) => {
            if (event.target.closest('a') || (event.target.closest('button') && !event.target.closest('[data-details]'))) return;

            selectMessage(message.id);
        });
        element.querySelectorAll('[data-tap]').forEach((button) => button.addEventListener('click', () => tap(message.id, Number(button.dataset.tap))));
        element.querySelectorAll('[data-status]').forEach((button) => button.addEventListener('click', () => fireStatus(message.id, button.dataset.status, null)));
        const failure = element.querySelector('[data-failure]');
        const sendFailure = element.querySelector('[data-send-failure]');
        failure?.addEventListener('change', () => { sendFailure.disabled = failure.value === ''; });
        sendFailure?.addEventListener('click', () => {
            if (failure.value === '') return;

            fireStatus(message.id, 'failed', failure.value);
        });
        list.append(element);
    }

    if (atBottom || view.scrollToEnd) {
        list.scrollTop = list.scrollHeight;
        view.scrollToEnd = false;
        list.querySelectorAll('img').forEach((image) => {
            if (!image.complete) image.addEventListener('load', () => { list.scrollTop = list.scrollHeight; }, { once: true });
        });
    }

    renderInspector();
}

function typeDetail(message) {
    if (message.direction === 'in') return message.context_id ? t('type.reply') : '';
    if (message.direction !== 'out' || ['template', 'reaction'].includes(message.type) || message.status === 'rejected') return '';

    return t(message.issues.some((issue) => issue.code === 'outside_service_window') ? 'type.outside' : 'type.inside');
}

function fact(label, value, className = '') {
    return `<div class="fact"><dt>${label}</dt><dd class="${className}">${value}</dd></div>`;
}

function renderInspector() {
    const message = view.conversation?.messages.find((candidate) => candidate.id === view.selectedId);
    $('#inspector').hidden = message === undefined;

    if (message === undefined) return;

    const app = appName(message.app_id);
    const out = message.direction === 'out';
    const lastStatus = message.statuses.at(-1);

    $('#inspector-title').textContent = t(message.status === 'rejected' ? 'inspector.rejected' : out ? 'inspector.app' : 'inspector.user');

    const webhooks = message.webhooks.map((webhook) => {
        const ok = webhook.status_code !== null && webhook.status_code < 300;
        const result = webhook.status_code === null ? t('inspector.noAnswer') : `${webhook.status_code}  ${webhook.duration_ms} ms`;

        return `<li><span class="when">${time(webhook.created_at)}</span><span class="what">${escapeHtml(webhook.event.replace(/^(status|message):/, ''))}</span><span class="code ${ok ? '' : 'bad'}">${escapeHtml(result)}</span><button type="button" class="link-button" data-webhook="${webhook.id}" data-key="webhook:${webhook.id}">${t('inspector.viewBody')}</button><button type="button" class="link-button" data-redeliver="${webhook.id}" data-key="redeliver:${webhook.id}">${t('inspector.redeliver')}</button></li>`;
    }).join('');

    const statusText = message.status === 'rejected'
        ? t('inspector.rejectedAt', { time: time(message.created_at) })
        : t('inspector.statusAt', { status: message.direction === 'in' ? inboundLabel(message) : statusLabel(message), time: time(lastStatus?.created_at ?? message.created_at) });

    $('#inspector-body').innerHTML = `
        <dl class="facts">
            ${fact(t('fact.id'), escapeHtml(message.id), 'mono')}
            ${fact(t('fact.type'), escapeHtml(message.type + typeDetail(message)))}
            ${fact(t('fact.status'), escapeHtml(statusText), 'strong')}
        </dl>
        ${buttonsDetail(message)}
        ${message.error ? `<section class="section"><h3>${escapeHtml(t('inspector.errorReturned', { app }))}</h3><pre>${escapeHtml(JSON.stringify(message.error, null, 2))}</pre></section>` : ''}
        <section class="section">
            <h3>${escapeHtml(t(out ? 'inspector.payloadSent' : 'inspector.webhookMessage', { app }))}</h3>
            <pre>${escapeHtml(JSON.stringify(message.payload, null, 2))}</pre>
        </section>
        ${webhooks ? `<section class="section"><h3>${escapeHtml(out ? t('inspector.statusesSent', { app }) : t('inspector.deliveries'))}</h3><ul class="log">${webhooks}</ul><div id="webhook-detail"></div></section>` : ''}`;

    if (view.webhookDetail?.messageId === message.id && $('#webhook-detail')) {
        $('#webhook-detail').innerHTML = view.webhookDetail.html;
    }

    $('#inspector-body').querySelectorAll('[data-webhook]').forEach((button) => button.addEventListener('click', async () => {
        try {
            const webhook = await api(`/webhooks/${button.dataset.webhook}`);
            let body = webhook.body;

            try { body = JSON.stringify(JSON.parse(body), null, 2); } catch { /* keep raw */ }

            const html = `
                <h3>${t('inspector.bodySent')}</h3><pre>${escapeHtml(body)}</pre>
                <h3>${escapeHtml(t('inspector.response', { app }))}</h3><pre>${escapeHtml(webhook.response ?? webhook.error ?? '')}</pre>`;
            view.webhookDetail = { messageId: message.id, html };
            $('#webhook-detail').innerHTML = html;
        } catch (error) {
            toast(error.message, true);
        }
    }));
    $('#inspector-body').querySelectorAll('[data-redeliver]').forEach((button) => button.addEventListener('click', () => redeliver(button.dataset.redeliver)));
}

async function redeliver(webhookId) {
    try {
        const webhook = await api(`/webhooks/${encodeURIComponent(webhookId)}/redeliver`, { method: 'POST', body: '{}' });

        if (webhook.status_code !== null && webhook.status_code < 300) {
            toast(t('toast.redelivered', { app: appName(webhook.app_id) }));
        } else {
            reportWebhook(webhook);
        }
    } catch (error) {
        toast(error.message, true);
    }
}

async function verifyWebhook(appId) {
    const app = appName(appId);

    try {
        const result = await api(`/apps/${encodeURIComponent(appId)}/verify-webhook`, { method: 'POST', body: '{}' });

        if (result.ok) {
            toast(t('toast.verifyOk', { app }));
        } else {
            const reason = result.error ?? (result.status_code === 200 ? t('verify.body') : t('verify.status', { status: result.status_code }));
            toast(t('toast.verifyFailed', { app, reason }), true);
        }
    } catch (error) {
        toast(error.message, true);
    }
}

function selectMessage(id) {
    view.selectedId = view.selectedId === id ? null : id;
    renderConversation();
}

async function refreshState() {
    view.state = await api('/state');
    renderBanner();
    renderSidebar();
}

async function refreshConversation() {
    if (view.current === null) {
        view.conversation = null;
    } else {
        try {
            view.conversation = await api(`/conversations/${encodeURIComponent(view.current.app)}/${encodeURIComponent(view.current.contact)}`);
        } catch (error) {
            if (error.status === 404) {
                view.current = null;
                view.conversation = null;
                history.replaceState(null, '', location.pathname);
                renderSidebar();
                renderConversation();
            }

            throw error;
        }

        if (view.conversation.messages.at(-1)?.direction === 'out') {
            $('#typing').hidden = true;
        }
    }

    renderConversation();
}

async function openConversation(app, contact) {
    view.current = { app, contact };
    view.selectedId = null;
    view.webhookDetail = null;
    view.scrollToEnd = true;
    clearTimeout(view.typingTimer);
    $('#typing').hidden = true;
    history.replaceState(null, '', `#${encodeURIComponent(app)}/${encodeURIComponent(contact)}`);
    renderSidebar();
    await refreshConversation();
    $('#text').focus();
}

function reportWebhook(webhook) {
    const app = appName(webhook.app_id);

    if (webhook.status_code === null) {
        toast(t('toast.noAnswer', { app, error: webhook.error }), true);
    } else if (webhook.status_code >= 300) {
        toast(t('toast.refused', { app, status: webhook.status_code }), true);
    }
}

async function sendInbound(app, contact, body) {
    const result = await api(`/conversations/${encodeURIComponent(app)}/${encodeURIComponent(contact)}/messages`, { method: 'POST', body: JSON.stringify(body) });
    reportWebhook(result.webhook);

    return result;
}

async function tap(messageId, index) {
    try {
        const result = await api(`/messages/${encodeURIComponent(messageId)}/tap`, { method: 'POST', body: JSON.stringify({ index }) });
        reportWebhook(result.webhook);
        view.scrollToEnd = true;
    } catch (error) {
        toast(error.message, true);
    }
}

async function fireStatus(messageId, status, errorCode) {
    try {
        const webhook = await api(`/messages/${encodeURIComponent(messageId)}/status`, { method: 'POST', body: JSON.stringify({ status, error_code: errorCode }) });
        reportWebhook(webhook);

        if (webhook.status_code !== null && webhook.status_code < 300) {
            toast(t(`toast.simulated.${status}`, { app: appName(webhook.app_id) }));
        }
    } catch (error) {
        toast(error.message, true);
    }
}

function openNewConversation(appId) {
    const apps = view.state.apps;
    const chosen = appId ?? apps[0]?.id;
    const select = $('#new-app');

    select.innerHTML = apps.map((app) => `<option value="${escapeHtml(app.id)}" ${app.id === chosen ? 'selected' : ''}>${escapeHtml(app.name)}</option>`).join('');
    $('#new-app-field').hidden = appId !== undefined;
    $('#new-error').hidden = true;
    $('#new-title').textContent = appId !== undefined ? t('newConversation.titleIn', { app: appName(appId) }) : t('page.newTitle');
    $('#new-conversation').showModal();
    $('#new-number').focus();
}

function readFile(file) {
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(file);
    });
}

function mediaKind(mimeType) {
    if (mimeType.startsWith('image/')) return mimeType === 'image/webp' ? 'sticker' : 'image';
    if (mimeType.startsWith('audio/')) return 'audio';
    if (mimeType.startsWith('video/')) return 'video';

    return 'document';
}

function clearAttachment() {
    view.attachment = null;
    $('#attach').value = '';
    $('#attachment').hidden = true;
}

$('#composer').addEventListener('submit', async (event) => {
    event.preventDefault();

    if (view.current === null) return;

    const text = $('#text').value.trim();
    const attachment = view.attachment;

    if (text === '' && attachment === null) return;

    $('#send').disabled = true;

    try {
        const body = attachment === null
            ? { kind: 'text', text }
            : { kind: mediaKind(attachment.type || 'application/octet-stream'), mime_type: attachment.type || 'application/octet-stream', filename: attachment.name, caption: text, data: await readFile(attachment) };

        await sendInbound(view.current.app, view.current.contact, body);
        $('#text').value = '';
        $('#text').style.height = '';
        clearAttachment();
        view.scrollToEnd = true;
    } catch (error) {
        toast(error.message, true);
    } finally {
        $('#send').disabled = false;
    }
});

$('#text').addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        $('#composer').requestSubmit();
    }
});

$('#text').addEventListener('input', (event) => {
    event.target.style.height = 'auto';
    event.target.style.height = `${event.target.scrollHeight}px`;
});

$('#attach').addEventListener('change', (event) => {
    const file = event.target.files?.[0] ?? null;

    if (file === null) {
        clearAttachment();

        return;
    }

    if (file.size > 16 * 1024 * 1024) {
        toast(t('toast.tooLarge'), true);
        clearAttachment();

        return;
    }

    view.attachment = file;
    $('#attachment-name').textContent = `${file.name} (${Math.ceil(file.size / 1024)} KB)`;
    $('#attachment').hidden = false;
});

$('#attachment-clear').addEventListener('click', clearAttachment);
$('#inspector-close').addEventListener('click', () => { view.selectedId = null; renderConversation(); });
$('#new-cancel').addEventListener('click', () => $('#new-conversation').close());
$('#empty-start').addEventListener('click', () => openNewConversation());

$('#new-conversation-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    const app = $('#new-app').value;
    const raw = $('#new-number').value.trim();
    const contact = raw.includes('.') ? raw.replace(/^([a-z]{2})\./i, (prefix) => prefix.toUpperCase()) : raw.replace(/\D/g, '');

    try {
        await sendInbound(app, contact, { kind: 'text', text: $('#new-text').value, name: $('#new-name').value });
        $('#new-conversation').close();
        $('#new-conversation-form').reset();
        await refreshState();
        await openConversation(app, contact);
    } catch (error) {
        $('#new-error').textContent = error.message;
        $('#new-error').hidden = false;
    }
});

$('#delete-conversation').addEventListener('click', async () => {
    if (view.current === null || !confirm(t('confirm.deleteConversation'))) return;

    try {
        await api(`/conversations/${encodeURIComponent(view.current.app)}/${encodeURIComponent(view.current.contact)}`, { method: 'DELETE' });
        view.current = null;
        view.conversation = null;
        view.selectedId = null;
        history.replaceState(null, '', location.pathname);
        $('#typing').hidden = true;
        await refreshState();
        renderConversation();
        toast(t('toast.deleted'));
    } catch (error) {
        toast(error.message, true);
    }
});

$('#clear-all').addEventListener('click', async () => {
    if (!confirm(t('confirm.clearAll'))) return;

    try {
        await api('/messages', { method: 'DELETE' });
        view.current = null;
        view.conversation = null;
        view.selectedId = null;
        history.replaceState(null, '', location.pathname);
        $('#typing').hidden = true;
        await refreshState();
        renderConversation();
    } catch (error) {
        toast(error.message, true);
    }
});

let pending = null;

function scheduleRefresh() {
    if (document.activeElement?.matches('[data-failure]')) {
        document.activeElement.addEventListener('blur', scheduleRefresh, { once: true });

        return;
    }

    clearTimeout(pending);
    pending = setTimeout(async () => {
        try {
            await refreshState();

            if (view.refreshConversation) {
                view.refreshConversation = false;
                await refreshConversation();
            }
        } catch (error) {
            toast(error.message, true);
        }
    }, 120);
}

function concernsCurrent(change) {
    return change.app_id === '*' || (view.current !== null && change.app_id === view.current.app && (change.contact === null || change.contact === view.current.contact));
}

const events = new EventSource('/api/events');
events.onmessage = (event) => {
    const change = JSON.parse(event.data);

    if (change.typing && view.current?.app === change.app_id && view.current?.contact === change.contact) {
        $('#typing').hidden = false;
        clearTimeout(view.typingTimer);
        view.typingTimer = setTimeout(() => { $('#typing').hidden = true; }, 6000);
    }

    if (concernsCurrent(change)) {
        view.refreshConversation = true;
    }

    scheduleRefresh();
};

function hashTarget() {
    const [app, raw] = location.hash.slice(1).split('/').map((part) => {
        try { return decodeURIComponent(part); } catch { return ''; }
    });
    const contact = raw && /^[\d+\s()-]+$/.test(raw) ? raw.replace(/\D/g, '') : raw;

    return [app, contact];
}

window.addEventListener('hashchange', () => {
    const [app, contact] = hashTarget();

    if (app && contact && (view.current?.app !== app || view.current?.contact !== contact)) {
        openConversation(app, contact).catch((error) => toast(error.message, true));
    }
});

translatePage();

(async () => {
    await refreshState();
    const apiUrl = view.state.api_url || `${location.protocol}//${location.hostname}:8027`;
    $('#api-url').textContent = apiUrl;

    try {
        const parsed = new URL(apiUrl);
        $('#api-port').textContent = `API :${parsed.port || (parsed.protocol === 'https:' ? '443' : '80')}`;
    } catch {
        $('#api-port').textContent = 'API';
    }
    const [app, contact] = hashTarget();

    if (app && contact) {
        await openConversation(app, contact);
    } else {
        renderConversation();
    }
})().catch((error) => toast(error.message, true));
