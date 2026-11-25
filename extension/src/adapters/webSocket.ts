import type { SocketConnection, SocketHandlers, SocketPort } from '../ports/socket';

/** The WebSocket to the library's relay. */
export class BrowserWebSocket implements SocketPort {
  open(url: string, handlers: SocketHandlers): SocketConnection {
    const ws = new WebSocket(url);
    const queue: string[] = [];
    ws.onopen = () => queue.splice(0).forEach((m) => ws.send(m));
    ws.onmessage = (e) => handlers.message(String(e.data));
    ws.onclose = () => handlers.close();
    return {
      send: (text) => (ws.readyState === WebSocket.OPEN ? ws.send(text) : queue.push(text)),
      close: () => ws.close(),
      isOpen: () => ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING,
    };
  }
}
