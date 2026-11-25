export interface Tab {
  id: number;
  url: string;
  title: string;
  windowId: number;
}

/** The browser's tabs. */
export interface TabsPort {
  list(): Promise<Tab[]>;
  get(id: number): Promise<Tab | undefined>;
  create(url: string, active: boolean): Promise<Tab>;
  remove(id: number): Promise<void>;
  activate(id: number): Promise<Tab>;
}
