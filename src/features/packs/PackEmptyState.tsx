import type { ReactNode } from 'react';

interface EmptyStateProps {
  /** Omit for a single-line panel, as in the loading state. */
  title?: string;
  children: ReactNode;
  action?: ReactNode;
}

/** The dashed panel for an empty or loading list. */
export function EmptyState({ title, children, action }: EmptyStateProps) {
  if (!title) {
    return <p className="packs-empty">{children}</p>;
  }
  return (
    <div className="packs-empty">
      <h3>{title}</h3>
      <p>{children}</p>
      {action}
    </div>
  );
}

interface StatusNoteProps {
  children: ReactNode;
  role?: 'status';
}

/** Muted inline copy for an empty list, a status line, or a blocking reason. */
export function StatusNote({ children, role }: StatusNoteProps) {
  return (
    <p className="empty-note" role={role}>
      {children}
    </p>
  );
}
