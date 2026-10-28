import { Gauge, Hash, Heading1, ListChecks, MousePointerClick, Table2, TextCursorInput, Type, type LucideIcon } from 'lucide-react';
import type { AppComponentKind } from '../../../api/apps/types';

/** The icon of each component kind (palette, outline, cards). */
export const KIND_ICON: Record<AppComponentKind, LucideIcon> = {
  heading: Heading1,
  text: Type,
  text_input: TextCursorInput,
  number_input: Hash,
  select: ListChecks,
  button: MousePointerClick,
  table: Table2,
  metric: Gauge,
};
