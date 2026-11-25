import { useEffect, useState } from 'react';
import { IDLE, type ConnectionStatus } from '../../domain/status';
import type { StatusClient } from '../../ports/status';
import { ConnectionCard } from '../organisms/ConnectionCard';
import { TabList } from '../organisms/TabList';
import { PopupLayout } from '../templates/PopupLayout';

/** The popup: whether the library is connected, and the tabs it drives. */
export function StatusPage({ client }: { client: StatusClient }) {
  const [status, setStatus] = useState<ConnectionStatus>(IDLE);
  useEffect(() => {
    let live = true;
    void client.status().then((s) => live && setStatus(s));
    const stop = client.subscribe(setStatus);
    return () => {
      live = false;
      stop();
    };
  }, [client]);
  return (
    <PopupLayout>
      <ConnectionCard status={status} onDisconnect={() => void client.disconnect()} />
      {status.state === 'connected' && <TabList tabs={status.tabs} />}
    </PopupLayout>
  );
}
