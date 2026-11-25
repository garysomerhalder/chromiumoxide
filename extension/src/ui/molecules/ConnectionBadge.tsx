import Stack from '@mui/material/Stack';
import { Label } from '../atoms/Label';
import { StatusDot } from '../atoms/StatusDot';

export function ConnectionBadge({ connected }: { connected: boolean }) {
  return (
    <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
      <StatusDot on={connected} />
      <Label kind="title">{connected ? 'Connected' : 'Not connected'}</Label>
    </Stack>
  );
}
