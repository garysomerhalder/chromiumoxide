import List from '@mui/material/List';
import type { TabSummary } from '../../domain/status';
import { Label } from '../atoms/Label';
import { TabItem } from '../molecules/TabItem';

export function TabList({ tabs }: { tabs: TabSummary[] }) {
  if (tabs.length === 0) return <Label kind="hint">No tabs are being driven.</Label>;
  return (
    <List dense disablePadding aria-label="Driven tabs">
      {tabs.map((t) => (
        <TabItem key={t.id} title={t.title} url={t.url} />
      ))}
    </List>
  );
}
