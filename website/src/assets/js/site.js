// Tulpa & HermiSim: site behaviour (no dependencies)
(() => {
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];
  const ROOT = window.SITE_ROOT || '';

  // ---------------------------------------------------------- copy buttons
  document.addEventListener('click', async (e) => {
    const btn = e.target.closest('.copy');
    if (!btn) return;
    const box = btn.closest('.code, .install-line');
    const code = box && (box.querySelector('pre code') || box.querySelector('code'));
    if (!code) return;
    const text = code.innerText.replace(/\n$/, '');
    try { await navigator.clipboard.writeText(text); } catch {
      const ta = Object.assign(document.createElement('textarea'), { value: text });
      document.body.append(ta); ta.select(); document.execCommand('copy'); ta.remove();
    }
    const old = btn.textContent; btn.textContent = 'Copied'; btn.classList.add('done');
    setTimeout(() => { btn.textContent = old; btn.classList.remove('done'); }, 1400);
  });

  // ---------------------------------------------------------- mobile menu
  $('[data-menu]')?.addEventListener('click', () => document.body.classList.toggle('menu-open'));

  // ---------------------------------------------------------- tabs
  for (const list of $$('[role="tablist"]')) {
    const tabs = $$('[role="tab"]', list);
    const select = (t, focus) => {
      for (const o of tabs) {
        const on = o === t;
        o.setAttribute('aria-selected', on); o.tabIndex = on ? 0 : -1;
        const p = document.getElementById(o.getAttribute('aria-controls'));
        if (p) p.hidden = !on;
      }
      if (focus) t.focus();
    };
    tabs.forEach((t, i) => {
      t.addEventListener('click', () => select(t));
      t.addEventListener('keydown', (e) => {
        if (e.key === 'ArrowRight') select(tabs[(i + 1) % tabs.length], true);
        if (e.key === 'ArrowLeft') select(tabs[(i - 1 + tabs.length) % tabs.length], true);
      });
    });
  }

  // ---------------------------------------------------------- reveal on scroll
  const io = 'IntersectionObserver' in window && new IntersectionObserver((es) => {
    for (const e of es) if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); }
  }, { rootMargin: '0px 0px -8% 0px' });
  $$('.reveal').forEach((el) => io ? io.observe(el) : el.classList.add('in'));

  // ---------------------------------------------------------- TOC scrollspy
  const tocLinks = $$('.docs-toc a');
  if (tocLinks.length) {
    const heads = tocLinks.map((a) => document.getElementById(decodeURIComponent(a.hash.slice(1)))).filter(Boolean);
    const onScroll = () => {
      let cur = heads[0];
      for (const h of heads) if (h.getBoundingClientRect().top < 120) cur = h;
      tocLinks.forEach((a) => a.classList.toggle('active', a.hash.slice(1) === cur?.id));
    };
    addEventListener('scroll', onScroll, { passive: true }); onScroll();
  }
  // keep the current sidebar item in view
  $('.side-group a[aria-current]')?.scrollIntoView({ block: 'center' });

  // ---------------------------------------------------------- lightbox
  document.addEventListener('click', (e) => {
    const img = e.target.closest('.gallery img, .prose .shot img, [data-zoom]');
    if (!img) return;
    const lb = Object.assign(document.createElement('div'), { className: 'lightbox' });
    lb.innerHTML = `<img src="${img.currentSrc || img.src}" alt="${img.alt}">`;
    const close = () => { lb.remove(); removeEventListener('keydown', esc); };
    const esc = (k) => k.key === 'Escape' && close();
    lb.addEventListener('click', close); addEventListener('keydown', esc);
    document.body.append(lb);
  });

  // ---------------------------------------------------------- search
  const box = $('[data-search]');
  const input = box && $('input', box);
  const results = box && $('.search-results', box);
  let index = null, sel = 0, items = [];
  const escHtml = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const mark = (s, terms) => {
    let out = escHtml(s);
    for (const t of terms) if (t.length > 1) out = out.replace(new RegExp(`(${t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'ig'), '<mark>$1</mark>');
    return out;
  };
  async function open() {
    box.hidden = false; input.value = ''; input.focus(); render();
    if (!index) { try { index = await (await fetch(ROOT + 'search-index.json')).json(); } catch { index = []; } render(); }
  }
  const close = () => { box.hidden = true; };
  function render() {
    const q = input.value.trim().toLowerCase();
    if (!index) { results.innerHTML = '<li class="search-empty">Loading…</li>'; return; }
    if (!q) {
      items = index.filter((d) => !d.h && d.s !== 'Site').slice(0, 8);
    } else {
      const terms = q.split(/\s+/);
      items = index.map((d) => {
        const t = d.t.toLowerCase(), x = (d.x || '').toLowerCase();
        let score = 0;
        for (const term of terms) {
          if (t === term) score += 30; else if (t.startsWith(term)) score += 18; else if (t.includes(term)) score += 10;
          else if (x.includes(term)) score += 2 + Math.min(4, x.split(term).length - 1);
          else return null;
        }
        if (d.h) score += 1;
        return { d, score };
      }).filter(Boolean).sort((a, b) => b.score - a.score).slice(0, 12).map((r) => r.d);
    }
    sel = 0;
    if (!items.length) { results.innerHTML = `<li class="search-empty">No results for “${escHtml(q)}”</li>`; return; }
    const terms = q ? q.split(/\s+/) : [];
    results.innerHTML = items.map((d, i) => {
      let ex = '';
      if (q && d.x) {
        const lx = d.x.toLowerCase(), at = lx.indexOf(terms[0]);
        if (at >= 0) ex = `<span class="ex">${mark((at > 40 ? '…' : '') + d.x.slice(Math.max(0, at - 40), at + 90), terms)}</span>`;
      }
      return `<li><a href="${ROOT + d.u}" class="${i === sel ? 'sel' : ''}"><small>${escHtml(d.s)}</small>${mark(d.t, terms)}${ex}</a></li>`;
    }).join('');
  }
  if (box) {
    $$('[data-search-open]').forEach((b) => b.addEventListener('click', open));
    box.addEventListener('click', (e) => { if (e.target === box) close(); });
    input.addEventListener('input', render);
    input.addEventListener('keydown', (e) => {
      const links = $$('a', results);
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        sel = (sel + (e.key === 'ArrowDown' ? 1 : -1) + links.length) % links.length;
        links.forEach((a, i) => a.classList.toggle('sel', i === sel));
        links[sel]?.scrollIntoView({ block: 'nearest' });
      } else if (e.key === 'Enter' && links[sel]) { location.href = links[sel].href; close(); }
    });
    addEventListener('keydown', (e) => {
      const typing = /INPUT|TEXTAREA|SELECT/.test(document.activeElement?.tagName);
      if (e.key === 'Escape' && !box.hidden) close();
      else if (!typing && (e.key === '/' || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k'))) { e.preventDefault(); open(); }
    });
  }
})();
