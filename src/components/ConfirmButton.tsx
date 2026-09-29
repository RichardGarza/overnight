// Two-click button: native confirm() dialogs are unreliable inside webviews,
// so the first click arms the button and the second one does the deed.

import { useEffect, useState } from "react";

interface Props {
  label: React.ReactNode;
  confirmLabel: React.ReactNode;
  onConfirm: () => void;
  className?: string;
  disabled?: boolean;
  /** Read by screen readers when the visible label is an icon. */
  ariaLabel?: string;
  title?: string;
}

export default function ConfirmButton({ label, confirmLabel, onConfirm, className, disabled, ariaLabel, title }: Props) {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), 3500);
    return () => clearTimeout(t);
  }, [armed]);
  return (
    <button
      type="button"
      className={`${className ?? "btn"} ${armed ? "armed" : ""}`}
      disabled={disabled}
      aria-label={armed ? `Confirm: ${ariaLabel ?? ""}`.trim() : ariaLabel}
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        if (armed) {
          setArmed(false);
          onConfirm();
        } else {
          setArmed(true);
        }
      }}
    >
      {armed ? confirmLabel : label}
    </button>
  );
}
