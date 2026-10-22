import { cn } from "@/lib/utils";

/**
 * A compact segmented control: native radios (so arrow keys move between the
 * choices) drawn as a row of pills in a tray, the chosen one raised.
 */
export function Segmented<T extends string>({
  name,
  label,
  options,
  value,
  onChange,
  className,
}: {
  name: string;
  label: string;
  options: { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
  className?: string;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cn("inline-flex items-center gap-0.5 rounded-lg bg-secondary p-0.5 ring-1 ring-border ring-inset", className)}
    >
      {options.map((option) => (
        <label
          key={option.value}
          className={cn(
            "relative cursor-pointer rounded-md px-2.5 py-1 text-xs font-medium whitespace-nowrap text-muted-foreground transition-colors select-none hover:text-foreground",
            "has-[:checked]:bg-background has-[:checked]:text-foreground has-[:checked]:shadow-xs has-[:checked]:ring-1 has-[:checked]:ring-border",
            "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring/50",
          )}
        >
          <input
            type="radio"
            name={name}
            value={option.value}
            checked={value === option.value}
            onChange={() => onChange(option.value)}
            className="sr-only"
          />
          {option.label}
        </label>
      ))}
    </div>
  );
}
