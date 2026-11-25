import Card from '@mui/material/Card';
import CardActions from '@mui/material/CardActions';
import CardContent from '@mui/material/CardContent';
import Stack from '@mui/material/Stack';
import type { ConnectionStatus } from '../../domain/status';
import { ActionButton } from '../atoms/ActionButton';
import { Label } from '../atoms/Label';
import { ConnectionBadge } from '../molecules/ConnectionBadge';

export function ConnectionCard({ status, onDisconnect }: { status: ConnectionStatus; onDisconnect: () => void }) {
  const connected = status.state === 'connected';
  return (
    <Card variant="outlined">
      <CardContent>
        <Stack spacing={1}>
          <ConnectionBadge connected={connected} />
          {connected ? (
            <Label kind="mono">{status.relayUrl}</Label>
          ) : (
            <Label kind="hint">
              Run Chromium::connect_extension() in your program: it opens a page here and this browser connects.
            </Label>
          )}
        </Stack>
      </CardContent>
      {connected && (
        <CardActions>
          <ActionButton onClick={onDisconnect}>Disconnect</ActionButton>
        </CardActions>
      )}
    </Card>
  );
}
