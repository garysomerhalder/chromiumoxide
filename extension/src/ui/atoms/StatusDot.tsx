import Box from '@mui/material/Box';

/** A small round light: green when connected, grey otherwise. */
export function StatusDot({ on }: { on: boolean }) {
  return (
    <Box
      component="span"
      aria-hidden
      sx={{
        display: 'inline-block',
        width: 10,
        height: 10,
        borderRadius: '50%',
        bgcolor: on ? 'success.main' : 'grey.400',
      }}
    />
  );
}
