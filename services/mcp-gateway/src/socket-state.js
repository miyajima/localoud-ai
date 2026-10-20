export function rejectSocketPending(pending, socket) {
  for (const [id, request] of pending) {
    if (socket && request.socket !== socket) continue;
    clearTimeout(request.timer);
    pending.delete(id);
    request.reject(new Error('device_offline'));
  }
}
