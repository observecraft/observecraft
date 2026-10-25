import { useEffect, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import './ui.css';

interface Props {
  onClose: () => void;
  label: string;
  children: ReactNode;
  width?: number | string;
  height?: number | string;
}

/** Centred dialog over a dimmed backdrop. Esc and backdrop click close it. */
export function Modal({ onClose, label, children, width = 1300, height = 'min(940px, 100%)' }: Props) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    const prev = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      window.removeEventListener('keydown', onKey);
      document.body.style.overflow = prev;
    };
  }, [onClose]);

  return createPortal(
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-label={label} style={{ width, height }}>
        {children}
      </div>
    </div>,
    document.body,
  );
}
