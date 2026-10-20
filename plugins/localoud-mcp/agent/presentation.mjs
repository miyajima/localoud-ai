// A display projection only. The approval gate remains the execution authority.
export function presentApprovals(jobs, now) {
  const pending = [], history = [];
  for (const job of jobs) {
    if (job.status === 'pending' && Number.isFinite(job.expires_at) && job.expires_at > now) {
      pending.push(job);
    } else {
      history.push(job.status === 'pending' ? { ...job, status: 'expired' } : job);
    }
  }
  pending.sort((a, b) => (a.created_at ?? 0) - (b.created_at ?? 0));
  history.sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0));
  return { pending, history };
}

export function approvalNotice(status) {
  if (status === 'completed') return { outcome: 'recorded', message: '処理結果を記録しました。ChatGPTで結果を確認できます。' };
  if (status === 'expired') return { outcome: 'not_executed', message: '期限切れのため実行していません。ChatGPTから新しい申請を送ってください。' };
  if (status === 'denied') return { outcome: 'not_executed', message: '拒否しました。実行していません。' };
  if (status === 'running') return { outcome: 'running', message: '実行中です。結果をお待ちください。' };
  if (status === 'failed') return { outcome: 'failed', message: '処理に失敗しました。ChatGPTで操作結果を確認してください。' };
  return { outcome: 'unknown', message: '実行結果を確定できません。再申請する前に、ChatGPTで操作結果と対象を確認してください。' };
}
