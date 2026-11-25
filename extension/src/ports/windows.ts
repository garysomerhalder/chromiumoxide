export interface WindowBounds {
  left: number;
  top: number;
  width: number;
  height: number;
  windowState: string;
}

/** The browser's windows. */
export interface WindowsPort {
  get(id: number): Promise<WindowBounds>;
  update(id: number, bounds: Partial<WindowBounds>): Promise<void>;
  focus(id: number): Promise<void>;
}
