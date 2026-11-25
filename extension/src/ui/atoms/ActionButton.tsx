import Button from '@mui/material/Button';
import type { ReactNode } from 'react';

export function ActionButton({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <Button variant="outlined" size="small" onClick={onClick}>
      {children}
    </Button>
  );
}
