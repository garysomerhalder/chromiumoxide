import Typography from '@mui/material/Typography';
import type { ReactNode } from 'react';

type Variant = 'title' | 'body' | 'hint' | 'mono';

const STYLE: Record<Variant, { variant: 'subtitle1' | 'body2' | 'caption'; mono?: boolean; color?: string }> = {
  title: { variant: 'subtitle1' },
  body: { variant: 'body2' },
  hint: { variant: 'caption', color: 'text.secondary' },
  mono: { variant: 'caption', mono: true, color: 'text.secondary' },
};

/** Text in one of the popup's few styles. */
export function Label({ kind = 'body', children, noWrap }: { kind?: Variant; children: ReactNode; noWrap?: boolean }) {
  const s = STYLE[kind];
  return (
    <Typography
      variant={s.variant}
      color={s.color}
      noWrap={noWrap}
      component="div"
      sx={s.mono ? { fontFamily: 'ui-monospace, monospace' } : undefined}
    >
      {children}
    </Typography>
  );
}
