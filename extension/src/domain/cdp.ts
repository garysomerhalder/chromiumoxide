// Messages between the library and the extension: Chrome's DevTools protocol.

export type Params = Record<string, unknown>;

export interface Command {
  id: number;
  method: string;
  params?: Params;
  sessionId?: string;
}

export type Outgoing =
  | { id: number; sessionId?: string; result: unknown }
  | { id: number; sessionId?: string; error: { code: number; message: string } }
  | { method: string; params: Params; sessionId?: string };

export function reply(cmd: Command, result: unknown): Outgoing {
  return cmd.sessionId === undefined
    ? { id: cmd.id, result: result ?? {} }
    : { id: cmd.id, sessionId: cmd.sessionId, result: result ?? {} };
}

export function failure(cmd: Command, e: unknown): Outgoing {
  const message = e instanceof Error ? e.message : String(e);
  const error = { code: -32000, message };
  return cmd.sessionId === undefined ? { id: cmd.id, error } : { id: cmd.id, sessionId: cmd.sessionId, error };
}

/** Commands the extension cannot send for the library. */
export class Unavailable extends Error {}
