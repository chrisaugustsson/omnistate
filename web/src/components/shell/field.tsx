import { useId, useLayoutEffect, useRef, type ComponentProps, type ReactNode } from "react";
import { cn } from "@/lib/utils";

// A form's field after the prototype (tasks/ui-redesign/v2/components.css,
// .field): its label over the control, "optional" muted after the label, a
// hint under it and an error in place of the hint. The control gets the id,
// the description and the invalid state through the render function.

export type ControlProps = {
  id: string;
  "aria-describedby"?: string;
  "aria-invalid"?: true;
};

export function Field({
  label,
  optional = false,
  hint,
  error,
  children,
  className,
}: {
  label: ReactNode;
  optional?: boolean;
  hint?: ReactNode;
  error?: ReactNode;
  children: (control: ControlProps) => ReactNode;
  className?: string;
}) {
  const id = useId();
  const noteId = useId();
  const note = error ?? hint;
  return (
    <div className={cn("flex flex-col gap-1.5 [&+&]:mt-4", className)}>
      <label htmlFor={id} className="w-fit font-medium text-ink">
        {label}
        {optional && <span className="font-normal text-ink-3"> optional</span>}
      </label>
      {children({ id, "aria-describedby": note ? noteId : undefined, "aria-invalid": error ? true : undefined })}
      {note && (
        <span id={noteId} role={error ? "alert" : undefined} className={cn("text-meta", error ? "text-danger-ink" : "text-ink-3")}>
          {note}
        </span>
      )}
    </div>
  );
}

/** The look of a text field, a textarea and a field select: 32px, a strong hairline, cobalt with a ring on focus. */
export const controlClass =
  "w-full min-w-0 rounded-control border border-line-strong bg-sheet px-2.5 text-ink transition-[border-color,box-shadow] duration-(--t-fast) ease-out-quint " +
  "placeholder:text-ink-3 hover:border-faint focus:border-brand focus:shadow-focus focus:outline-none aria-invalid:border-danger disabled:bg-frame disabled:text-ink-3";

export function Input({ className, ...props }: ComponentProps<"input">) {
  return <input {...props} className={cn(controlClass, "h-(--control-h-lg)", className)} />;
}

/**
 * A textarea that grows with its text, from `minRows` up, so it never shows
 * an inner scrollbar.
 */
export function Textarea({ className, minRows = 3, value, ...props }: ComponentProps<"textarea"> & { minRows?: number }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const box = ref.current;
    if (!box) return;
    box.style.height = "auto";
    if (box.scrollHeight > 0) box.style.height = `${box.scrollHeight + 2}px`;
  }, [value]);
  return (
    <textarea
      ref={ref}
      rows={minRows}
      value={value}
      {...props}
      className={cn(controlClass, "resize-none overflow-hidden py-2 leading-(--lh-body)", className)}
    />
  );
}
