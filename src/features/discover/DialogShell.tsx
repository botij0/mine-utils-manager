import { useEffect, useRef, type ReactNode } from 'react';

interface DialogShellProps {
  label: string;
  onClose: () => void;
  children: ReactNode;
}

/**
 * One modal dialog at a time: `aria-modal`, initial focus in the dialog,
 * focus restored to the opener on unmount, and Escape to close. Callers
 * render at most one `DialogShell` so two sheets never stack.
 */
export default function DialogShell({ label, onClose, children }: DialogShellProps) {
  const dialogRef = useRef<HTMLElement | null>(null);
  const openerRef = useRef<Element | null>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    openerRef.current = document.activeElement;
    dialogRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation();
        closeRef.current();
      }
    };
    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('keydown', onKey, true);
      if (openerRef.current instanceof HTMLElement) {
        openerRef.current.focus();
      }
    };
  }, []);

  // Focus trap: keep Tab inside the dialog while it is open.
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key !== 'Tab' || !dialogRef.current) return;
    const focusable = Array.from(
      dialogRef.current.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      ),
    ).filter((el) => !el.hasAttribute('disabled'));
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div className="sheet-backdrop">
      {/* Focused on mount so keyboard users land inside the dialog. */}
      <section
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        ref={dialogRef}
        onKeyDown={onKeyDown}
      >
        {children}
      </section>
    </div>
  );
}
