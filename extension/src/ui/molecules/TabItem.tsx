import ListItem from '@mui/material/ListItem';
import ListItemText from '@mui/material/ListItemText';
import { Label } from '../atoms/Label';

/** One tab the library is driving. */
export function TabItem({ title, url }: { title: string; url: string }) {
  return (
    <ListItem disableGutters dense>
      <ListItemText
        disableTypography
        primary={<Label noWrap>{title || 'Untitled'}</Label>}
        secondary={<Label kind="mono" noWrap>{url}</Label>}
      />
    </ListItem>
  );
}
