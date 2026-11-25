import Box from '@mui/material/Box';
import CssBaseline from '@mui/material/CssBaseline';
import Stack from '@mui/material/Stack';
import { ThemeProvider } from '@mui/material/styles';
import type { ReactNode } from 'react';
import { theme } from '../theme';
import { Label } from '../atoms/Label';

/** The popup's frame: a title, then its sections. */
export function PopupLayout({ children }: { children: ReactNode }) {
  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <Box sx={{ width: 340, p: 2 }}>
        <Stack spacing={2}>
          <Label kind="hint">chromiumoxide</Label>
          {children}
        </Stack>
      </Box>
    </ThemeProvider>
  );
}
