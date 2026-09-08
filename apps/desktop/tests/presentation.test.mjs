import { test } from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
const dom = new JSDOM('<!doctype html><body></body>');
globalThis.window = dom.window;
globalThis.document = dom.window.document;
const { markdown, diffMarkup, planPreview } = await import('../src/presentation.ts');
test('agent content cannot introduce active HTML or unsafe links', () => {
  const html = markdown('<script>alert(1)</script><img src=x onerror=alert(1)>[click](javascript:alert(1))<a href="file:///etc/passwd">local</a>\n\n**result**\n\n```html\n<img src=x>\n```');
  const root=document.createElement('div'); root.innerHTML=html;
  assert.equal(root.querySelector('script,img,iframe,style'),null);
  assert.equal(root.querySelector('a[href^="javascript:"],a[href^="file:"]'),null);
  assert.equal(root.querySelector('strong').textContent,'result');
  assert.equal(root.querySelector('pre code').textContent.trim(),'<img src=x>');
});
test('web links keep explicit external handling; remote images never load', () => {
  const html=markdown('[Docs](https://example.com/docs) ![tracking](https://example.com/pixel)');
  const root=document.createElement('div');root.innerHTML=html;
  assert.equal(root.querySelector('a').dataset.externalUrl,'https://example.com/docs');
  assert.equal(root.querySelector('img'),null);
});
test('diff counts exclude headers and escape file content', () => {
  const html=diffMarkup('diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+<script>new</script>');
  assert.match(html,/class="added">\+1</);assert.match(html,/class="removed">−1</);
  assert.match(html,/&lt;script&gt;new&lt;\/script&gt;/);assert.ok(!html.includes('<script>'));
});
test('plan preview degrades safely for malformed or incomplete model output', () => {
  assert.match(planPreview('{'),/編集中/);
  assert.match(planPreview('{"steps":[null]}'),/編集中/);
  const html=planPreview(JSON.stringify({title:'<img src=x>',steps:[{key:'one',title:'first',goal:'<script>',dependencies:[],brief:{acceptance_criteria:['passes']}}]}));
  assert.ok(!html.includes('<script>'));assert.ok(!html.includes('<img'));assert.match(html,/passes/);
});
