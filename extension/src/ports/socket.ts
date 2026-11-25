export interface SocketHandlers {
  message(text: string): void;
  close(): void;
}

export interface SocketConnection {
  send(text: string): void;
  close(): void;
  isOpen(): boolean;
}

/** Opens the WebSocket to the library's relay. */
export interface SocketPort {
  open(url: string, handlers: SocketHandlers): SocketConnection;
}
