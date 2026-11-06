/* Shared authentication UI: user menu, admin links, change-password modal. */
(function () {
  const css = `
  .tp-user { position: relative; margin-left: .6rem; display: flex; align-items: center; }
  .tp-user-btn { display: flex; align-items: center; gap: .5rem; background: rgba(255,255,255,.08); border: 1px solid rgba(255,255,255,.14); color: #e2e8f0; padding: .28rem .55rem .28rem .32rem; border-radius: 999px; cursor: pointer; font: inherit; font-size: .78rem; }
  .tp-user-btn:hover { background: rgba(255,255,255,.14); }
  .tp-avatar { width: 24px; height: 24px; border-radius: 50%; background: #2563eb; color: #fff; display: flex; align-items: center; justify-content: center; font-weight: 800; font-size: .72rem; text-transform: uppercase; }
  .tp-role { font-size: .58rem; text-transform: uppercase; letter-spacing: .06em; color: #94a3b8; font-weight: 700; }
  .tp-user-menu { position: absolute; top: calc(100% + 8px); right: 0; min-width: 216px; background: #fff; color: #0f172a; border: 1px solid #e2e8f0; border-radius: 12px; box-shadow: 0 12px 30px rgba(15,23,42,.22); padding: 6px; display: none; z-index: 2000; }
  .tp-user-menu.open { display: block; }
  .tp-menu-head { padding: 8px 10px; border-bottom: 1px solid #f1f5f9; margin-bottom: 4px; }
  .tp-menu-head .name { font-weight: 800; font-size: .82rem; }
  .tp-menu-head .role { font-size: .64rem; color: #64748b; text-transform: uppercase; letter-spacing: .06em; margin-top: 1px; }
  .tp-user-menu a, .tp-user-menu button { display: flex; align-items: center; width: 100%; padding: 8px 10px; border: 0; background: none; color: #334155; font: inherit; font-size: .8rem; text-align: left; border-radius: 8px; cursor: pointer; text-decoration: none; }
  .tp-user-menu a:hover, .tp-user-menu button:hover { background: #f1f5f9; color: #0f172a; }
  .tp-user-menu .danger { color: #b91c1c; }
  .tp-user-menu .danger:hover { background: #fef2f2; color: #991b1b; }
  .tp-modal-backdrop { position: fixed; inset: 0; background: rgba(15,23,42,.55); z-index: 3000; display: none; align-items: center; justify-content: center; padding: 16px; }
  .tp-modal-backdrop.open { display: flex; }
  .tp-modal { width: 100%; max-width: 390px; background: #fff; color: #0f172a; border-radius: 14px; box-shadow: 0 20px 60px rgba(15,23,42,.3); padding: 18px; }
  .tp-modal h3 { font-size: 1rem; margin-bottom: 3px; }
  .tp-modal p.sub { font-size: .78rem; color: #64748b; margin-bottom: 14px; line-height: 1.45; }
  .tp-field { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
  .tp-field label { font-size: .62rem; font-weight: 800; text-transform: uppercase; letter-spacing: .06em; color: #64748b; }
  .tp-field input { padding: .55rem .6rem; border: 1px solid #e2e8f0; border-radius: 8px; font: inherit; font-size: .85rem; outline: none; }
  .tp-field input:focus { border-color: #2563eb; box-shadow: 0 0 0 3px rgba(37,99,235,.1); }
  .tp-modal-actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 14px; }
  .tp-modal-actions button { padding: .5rem .9rem; border-radius: 8px; border: 0; font: inherit; font-size: .8rem; font-weight: 700; cursor: pointer; }
  .tp-modal-actions .cancel { background: #e2e8f0; color: #334155; }
  .tp-modal-actions .save { background: #2563eb; color: #fff; }
  .tp-modal-actions .save:hover { background: #1d4ed8; }
  .tp-modal-actions .save:disabled { opacity: .6; cursor: wait; }
  .tp-modal-error { display: none; margin-top: 8px; padding: 8px 10px; border-radius: 8px; background: #fef2f2; color: #b91c1c; font-size: .76rem; line-height: 1.4; }
  .tp-modal-error.show { display: block; }
  .tp-toast { position: fixed; left: 50%; bottom: 26px; transform: translateX(-50%) translateY(10px); background: #0f172a; color: #fff; padding: .6rem 1rem; border-radius: 10px; font-size: .82rem; font-weight: 600; box-shadow: 0 10px 30px rgba(15,23,42,.3); opacity: 0; pointer-events: none; transition: all .22s; z-index: 4000; max-width: 90vw; }
  .tp-toast.show { opacity: 1; transform: translateX(-50%) translateY(0); }
  @media (max-width: 720px) { .tp-name, .tp-role { display: none; } .tp-user-btn { padding-right: .32rem; } }
  `;

  const style = document.createElement('style');
  style.textContent = css;
  document.head.appendChild(style);

  let toastTimer = null;
  window.tpToast = function (message) {
    let el = document.querySelector('.tp-toast');
    if (!el) {
      el = document.createElement('div');
      el.className = 'tp-toast';
      document.body.appendChild(el);
    }
    el.textContent = message;
    el.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.classList.remove('show'), 3200);
  };

  function escapeHtml(value) {
    return String(value == null ? '' : value)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function addAdminLinks(user) {
    if (user.role !== 'admin') return;
    const nav = document.querySelector('.nav');
    if (nav) {
      if (!nav.querySelector('a[href="/settings"]')) {
        nav.insertAdjacentHTML('beforeend', '<a href="/settings">Settings</a><a href="/users">Users</a>');
      }
      return;
    }
    const header = document.querySelector('body > header');
    if (header && !header.querySelector('a[href="/settings"]')) {
      header.insertAdjacentHTML('beforeend', '<a href="/settings">Settings</a><a href="/users">Users</a>');
    }
  }

  function buildMenu(user) {
    const roleLabel = user.role === 'admin' ? 'Administrator' : 'Viewer';
    const initial = escapeHtml((user.username || '?').slice(0, 1));
    const adminItems = user.role === 'admin'
      ? '<a href="/settings">Settings</a><a href="/users">Users</a>'
      : '';
    return `
      <div class="tp-user">
        <button class="tp-user-btn" id="tp-user-btn" type="button" aria-haspopup="true" aria-expanded="false">
          <span class="tp-avatar">${initial}</span>
          <span class="tp-name">${escapeHtml(user.username)}</span>
          <span class="tp-role">${roleLabel}</span>
        </button>
        <div class="tp-user-menu" id="tp-user-menu" role="menu">
          <div class="tp-menu-head">
            <div class="name">${escapeHtml(user.username)}</div>
            <div class="role">${roleLabel}</div>
          </div>
          ${adminItems}
          <button id="tp-change-pass" type="button">Change password</button>
          <button id="tp-logout" class="danger" type="button">Sign out</button>
        </div>
      </div>`;
  }

  function buildPasswordModal() {
    return `
      <div class="tp-modal-backdrop" id="tp-pass-backdrop">
        <div class="tp-modal" role="dialog" aria-modal="true" aria-labelledby="tp-pass-title">
          <h3 id="tp-pass-title">Change password</h3>
          <p class="sub">Enter your current password and choose a new one (at least 8 characters).</p>
          <div class="tp-field">
            <label for="tp-cur-pass">Current password</label>
            <input id="tp-cur-pass" type="password" autocomplete="current-password" />
          </div>
          <div class="tp-field">
            <label for="tp-new-pass">New password</label>
            <input id="tp-new-pass" type="password" autocomplete="new-password" />
          </div>
          <div class="tp-field">
            <label for="tp-confirm-pass">Confirm new password</label>
            <input id="tp-confirm-pass" type="password" autocomplete="new-password" />
          </div>
          <div class="tp-modal-error" id="tp-pass-error"></div>
          <div class="tp-modal-actions">
            <button class="cancel" id="tp-pass-cancel" type="button">Cancel</button>
            <button class="save" id="tp-pass-save" type="button">Update password</button>
          </div>
        </div>
      </div>`;
  }

  function wireMenu() {
    const btn = document.getElementById('tp-user-btn');
    const menu = document.getElementById('tp-user-menu');
    if (!btn || !menu) return;

    btn.addEventListener('click', (event) => {
      event.stopPropagation();
      const open = menu.classList.toggle('open');
      btn.setAttribute('aria-expanded', String(open));
    });
    document.addEventListener('click', () => {
      menu.classList.remove('open');
      btn.setAttribute('aria-expanded', 'false');
    });
    menu.addEventListener('click', (event) => event.stopPropagation());

    document.getElementById('tp-logout').addEventListener('click', async () => {
      try {
        await fetch('/api/auth/logout', { method: 'POST' });
      } catch (_) { /* ignore */ }
      window.location.href = '/login';
    });

    const backdrop = document.getElementById('tp-pass-backdrop');
    const openModal = () => {
      backdrop.classList.add('open');
      document.getElementById('tp-cur-pass').value = '';
      document.getElementById('tp-new-pass').value = '';
      document.getElementById('tp-confirm-pass').value = '';
      document.getElementById('tp-pass-error').classList.remove('show');
      setTimeout(() => document.getElementById('tp-cur-pass').focus(), 30);
    };
    const closeModal = () => backdrop.classList.remove('open');
    const showError = (message) => {
      const el = document.getElementById('tp-pass-error');
      el.textContent = message;
      el.classList.add('show');
    };

    document.getElementById('tp-change-pass').addEventListener('click', openModal);
    document.getElementById('tp-pass-cancel').addEventListener('click', closeModal);
    backdrop.addEventListener('click', (event) => {
      if (event.target === backdrop) closeModal();
    });
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') closeModal();
    });

    const save = document.getElementById('tp-pass-save');
    const submit = async () => {
      const current = document.getElementById('tp-cur-pass').value;
      const next = document.getElementById('tp-new-pass').value;
      const confirm = document.getElementById('tp-confirm-pass').value;
      document.getElementById('tp-pass-error').classList.remove('show');

      if (!current || !next) return showError('Fill in all password fields.');
      if (next.length < 8) return showError('New password must be at least 8 characters.');
      if (next !== confirm) return showError('The new passwords do not match.');

      save.disabled = true;
      save.textContent = 'Updating…';
      try {
        const res = await fetch('/api/auth/password', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ current_password: current, new_password: next }),
        });
        const data = await res.json().catch(() => ({}));
        if (!res.ok) {
          showError(data.error || 'Could not update password.');
          return;
        }
        closeModal();
        window.tpToast('Password updated');
      } catch (_) {
        showError('Could not reach the server.');
      } finally {
        save.disabled = false;
        save.textContent = 'Update password';
      }
    };

    save.addEventListener('click', submit);
    ['tp-cur-pass', 'tp-new-pass', 'tp-confirm-pass'].forEach((id) => {
      document.getElementById(id).addEventListener('keydown', (event) => {
        if (event.key === 'Enter') submit();
      });
    });
  }

  function updateBrand(platformName) {
    if (!platformName || platformName === 'TrackPulse') return;
    document.title = document.title.replace(/^TrackPulse/, platformName);
    document.querySelectorAll('.brand').forEach((el) => {
      let replaced = false;
      for (const node of el.childNodes) {
        if (node.nodeType === Node.TEXT_NODE && node.textContent.trim()) {
          node.textContent = platformName;
          replaced = true;
          break;
        }
      }
      if (!replaced) el.appendChild(document.createTextNode(platformName));
    });
    const legacy = document.querySelector('body > header > div:first-child');
    if (legacy && legacy.textContent.trim() === 'TrackPulse') {
      legacy.textContent = platformName;
    }
  }

  (async function init() {
    let user = null;
    try {
      const res = await fetch('/api/auth/me', { headers: { Accept: 'application/json' } });
      if (res.status === 401) {
        window.location.href = '/login';
        return;
      }
      if (res.ok) user = await res.json();
    } catch (_) {
      return;
    }
    if (!user) return;

    window.TP_USER = user;
    addAdminLinks(user);

    const host = document.querySelector('.topbar-right') || document.querySelector('body > header');
    if (host) host.insertAdjacentHTML('beforeend', buildMenu(user));

    document.body.insertAdjacentHTML('beforeend', buildPasswordModal());
    wireMenu();

    try {
      const res = await fetch('/api/settings/public');
      if (res.ok) {
        const data = await res.json();
        updateBrand(data.platform_name);
      }
    } catch (_) { /* ignore */ }
  })();
})();
