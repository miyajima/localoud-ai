(() => {
  // Observe completion metadata only. Never read prompts, answers, cookies or tokens.
  if (location.hostname !== 'chatgpt.com' || window.top !== window) return;
  const messages = () => Array.from(document.querySelectorAll('[data-message-author-role="assistant"][data-message-id]'));
  const ids = () => new Set(messages().map(node => node.getAttribute('data-message-id')));
  const modelName = value => /\bastra\b/i.test(value || '') ? 'astra' : /\bsol\b/i.test(value || '') ? 'sol' : 'unknown';
  const selectedModel = () => modelName(document.querySelector('[data-testid="model-switcher-dropdown-button"]')?.textContent);
  let baseline = ids(), pending = null, quietSince = null, wasBusy = false;
  const tick = () => {
    if (pending?.path?.startsWith('/c/') && location.pathname !== pending.path) {
      pending = null; quietSince = null; baseline = ids(); wasBusy = false;
    }
    const busy = !!document.querySelector('[data-testid="stop-button"], [data-testid="stop-generating-button"]');
    if (busy) {
      if (!wasBusy && !pending) pending = {started: Date.now(), model: selectedModel(), baseline, path: location.pathname};
      quietSince = null;
    } else if (pending) {
      quietSince ??= Date.now();
      if (Date.now() - quietSince >= 1200) {
        const last = messages().filter(node => !pending.baseline.has(node.getAttribute('data-message-id'))).at(-1);
        if (last) {
          const id = last.getAttribute('data-message-id');
          const reported = modelName(last.getAttribute('data-message-model-slug'));
          const model = reported === 'unknown' ? pending.model : reported;
          if (/^[a-zA-Z0-9_-]{1,128}$/.test(id)) {
            const query = new URLSearchParams({id, model, duration: String(Math.max(0, quietSince - pending.started))});
            location.href = `localoud-usage://completed?${query}`;
          }
        }
        pending = null;
        baseline = ids();
      }
    } else {
      // Loaded history is a baseline, never a completed turn.
      baseline = ids();
    }
    wasBusy = busy;
  };
  const start = () => {
    baseline = ids();
    document.addEventListener('submit', event => {
      if (event.target?.querySelector?.('#prompt-textarea') && !pending) {
        pending = {started: Date.now(), model: selectedModel(), baseline: ids(), path: location.pathname};
        quietSince = null;
      }
    }, true);
    new MutationObserver(tick).observe(document.documentElement, {childList:true, subtree:true, attributes:true, attributeFilter:['data-testid','data-message-id']});
    setInterval(tick, 300);
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, {once:true}); else start();
})();
