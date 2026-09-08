import { marked } from 'marked';
import DOMPurify from 'dompurify';

export const escapeHtml = (s: string) => s.replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]!));
export function markdown(text: string): string {
  const html = DOMPurify.sanitize(marked.parse(text, { async: false, breaks: true }), {
    ALLOWED_TAGS: ['p','br','strong','em','del','code','pre','ul','ol','li','h1','h2','h3','h4','blockquote','hr','table','thead','tbody','tr','th','td','a'],
    ALLOWED_ATTR: ['href','title'],
  });
  const template = document.createElement('template');
  template.innerHTML = html;
  template.content.querySelectorAll('a').forEach(a => {
    const href = a.getAttribute('href') || '';
    if (!/^https?:\/\//i.test(href)) { a.removeAttribute('href'); return; }
    a.dataset.externalUrl = href; a.title = `${href} — ブラウザで開く`;
  });
  return template.innerHTML;
}
export function diffMarkup(diff: string): string {
  const lines = diff.split('\n');
  const added = lines.filter(l => l.startsWith('+') && !l.startsWith('+++')).length;
  const removed = lines.filter(l => l.startsWith('-') && !l.startsWith('---')).length;
  return `<div class="diff-stats"><span class="added">+${added}</span><span class="removed">−${removed}</span><span>行の変更</span></div><pre class="diff">${lines.map(l => `<span class="diff-line ${l.startsWith('+++') || l.startsWith('---') || l.startsWith('diff ') ? 'diff-file' : l.startsWith('+') ? 'diff-add' : l.startsWith('-') ? 'diff-remove' : l.startsWith('@@') ? 'diff-hunk' : ''}">${escapeHtml(l) || ' '}</span>`).join('')}</pre>`;
}
export function statusLabel(status: string): string {
  return ({idle:'待機中',completed:'完了',running:'実行中',inProgress:'実行中',pending:'待機',ready:'準備完了',reviewing:'レビュー中',blocked:'依存待ち',failed:'失敗',cancelled:'中止',interrupted:'中断'})[status] || status;
}
export function planPreview(raw: string): string {
  try {
    const plan = JSON.parse(raw) as {title?: string; steps?: {key:string;title:string;goal:string;dependencies:string[];brief?:{acceptance_criteria?:string[]}}[]};
    if (!Array.isArray(plan.steps)) return '<p class="muted">計画を作成すると、ステップをここに表示します。</p>';
    return `<h3>${escapeHtml(plan.title || '実行計画')}</h3><ol class="plan-preview">${plan.steps.map(s => `<li><strong>${escapeHtml(s.title || s.key)}</strong><p>${escapeHtml(s.goal || '')}</p><small>先に完了するタスク: ${escapeHtml(s.dependencies?.map(k=>plan.steps?.find(d=>d.key===k)?.title||k).join('、') || 'なし')}</small><ul>${(s.brief?.acceptance_criteria || []).map(c=>`<li>${escapeHtml(c)}</li>`).join('')}</ul></li>`).join('')}</ol>`;
  } catch { return '<p class="muted">計画 JSON の編集中です。実行前に形式を確認してください。</p>'; }
}
