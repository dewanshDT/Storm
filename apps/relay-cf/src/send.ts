// The only place this relay calls WebSocket.send. workerd throws on a socket
// that is closing or closed, and a peer can go at any moment, so a refused
// send is the peer's departure, never an exception: the caller ends that peer
// as its close event would.
export function trySend(ws: WebSocket, data: string | ArrayBuffer): boolean {
  if (ws.readyState !== WebSocket.READY_STATE_OPEN) return false;
  try {
    ws.send(data);
    return true;
  } catch {
    return false;
  }
}
