import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { layer } from "./dialog";
import { controlClass, type ControlProps } from "./field";

// A select drawn as a menu button (the select-only combobox of the ARIA
// patterns): a button showing the chosen option, and a listbox in the menu's
// look (tasks/ui-redesign/v2/components.css, .menu) under it. The focus stays
// on the button; arrows, Home and End move through the options, Enter or Space
// picks one, a letter jumps to the next option starting with it, Esc and Tab
// close it. The list is drawn on the page so a dialog's edges do not clip it.
//
// Two looks: "field", a 32px field of a form, and "filter", the quiet chip of a
// filter bar that is lit while it holds a value.

export type SelectOption<Value extends string = string> = {
  value: Value;
  label: string;
  /** A muted word after the label: a kind, a count. */
  hint?: string;
  /** A muted line under the label. */
  description?: string;
  icon?: ReactNode;
};

type Props<Value extends string> = Partial<ControlProps> & {
  value: Value | "";
  onChange: (value: Value) => void;
  options: readonly SelectOption<Value>[];
  /** Shown while no option is chosen. */
  placeholder?: string;
  look?: "field" | "filter";
  /** The name when no label points at it (a filter has none). */
  "aria-label"?: string;
  disabled?: boolean;
  className?: string;
};

type Place = { left: number; width: number; top?: number; bottom?: number; maxHeight: number };

export function Select<Value extends string>({
  value,
  onChange,
  options,
  placeholder = "Choose…",
  look = "field",
  disabled,
  className,
  id,
  ...aria
}: Props<Value>) {
  const listId = useId();
  const optionId = (index: number) => `${listId}-${index}`;
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const [place, setPlace] = useState<Place | null>(null);
  const chosen = options.findIndex((option) => option.value === value);
  const current = chosen >= 0 ? options[chosen] : undefined;

  function show(at = chosen >= 0 ? chosen : 0) {
    if (disabled || options.length === 0) return;
    setActive(at);
    setOpen(true);
  }
  function pick(index: number) {
    const option = options[index];
    setOpen(false);
    if (option && option.value !== value) onChange(option.value);
  }

  useLayoutEffect(() => {
    if (!open || !button.current) return;
    const box = button.current.getBoundingClientRect();
    const below = window.innerHeight - box.bottom - 8;
    const above = box.top - 8;
    const width = Math.max(box.width, look === "filter" ? 180 : 0);
    const left = Math.min(box.left, window.innerWidth - width - 8);
    setPlace(
      below >= 200 || below >= above
        ? { left, width, top: box.bottom + 4, maxHeight: Math.min(320, below) }
        : { left, width, bottom: window.innerHeight - box.top + 4, maxHeight: Math.min(320, above) },
    );
  }, [open, look]);

  useEffect(() => {
    if (!open) return;
    function outside(event: PointerEvent) {
      const target = event.target as Node;
      if (!button.current?.contains(target) && !list.current?.contains(target)) setOpen(false);
    }
    function scrolled(event: Event) {
      if (!list.current?.contains(event.target as Node)) setOpen(false);
    }
    const resized = () => setOpen(false);
    document.addEventListener("pointerdown", outside);
    window.addEventListener("scroll", scrolled, true);
    window.addEventListener("resize", resized);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("scroll", scrolled, true);
      window.removeEventListener("resize", resized);
    };
  }, [open]);

  useEffect(() => {
    if (open && active >= 0) document.getElementById(`${listId}-${active}`)?.scrollIntoView?.({ block: "nearest" });
  }, [open, active, listId]);

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    const last = options.length - 1;
    if (!open) {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(event.key)) {
        event.preventDefault();
        show(event.key === "ArrowUp" && chosen < 0 ? last : undefined);
      }
      return;
    }
    switch (event.key) {
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        return;
      case "Tab":
        setOpen(false);
        return;
      case "ArrowDown":
        event.preventDefault();
        setActive((now) => Math.min(last, now + 1));
        return;
      case "ArrowUp":
        event.preventDefault();
        setActive((now) => Math.max(0, now - 1));
        return;
      case "Home":
        event.preventDefault();
        setActive(0);
        return;
      case "End":
        event.preventDefault();
        setActive(last);
        return;
      case "Enter":
      case " ":
        event.preventDefault();
        pick(active);
        return;
    }
    if (event.key.length === 1 && /\S/.test(event.key)) {
      const letter = event.key.toLowerCase();
      const order = [...options.keys()].map((index) => (active + 1 + index) % options.length);
      const next = order.find((index) => options[index].label.toLowerCase().startsWith(letter));
      if (next !== undefined) setActive(next);
    }
  }

  return (
    <>
      <button
        ref={button}
        id={id}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={open && active >= 0 ? optionId(active) : undefined}
        disabled={disabled}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={onKeyDown}
        {...aria}
        className={cn(
          look === "field"
            ? cn(controlClass, "flex h-(--control-h-lg) items-center gap-2 pr-2 text-left aria-expanded:border-brand aria-expanded:shadow-focus")
            : cn(
                "inline-flex h-(--control-h) max-w-60 items-center gap-1 rounded-control pr-1.5 pl-2 font-medium transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none",
                value ? "bg-selected text-ink" : "text-ink-2",
              ),
          className,
        )}
      >
        {current?.icon && <span className="inline-flex shrink-0 [&_svg]:size-(--icon-sm)">{current.icon}</span>}
        <span className={cn("min-w-0 truncate", look === "field" && "flex-1", !current && "text-ink-3")}>
          {current?.label ?? placeholder}
          {look === "field" && current?.hint && <span className="text-ink-3"> · {current.hint}</span>}
        </span>
        <ChevronDown aria-hidden className={cn("shrink-0 text-ink-3", look === "field" ? "size-(--icon-sm)" : "size-3")} />
      </button>
      {open &&
        place &&
        createPortal(
          <div
            ref={list}
            id={listId}
            role="listbox"
            aria-label={aria["aria-label"] ?? button.current?.labels?.[0]?.textContent ?? undefined}
            onMouseDown={(event) => event.preventDefault()}
            style={{ left: place.left, top: place.top, bottom: place.bottom, minWidth: place.width, maxHeight: place.maxHeight }}
            className="fixed z-[70] max-w-[min(420px,calc(100vw-16px))] origin-top animate-pop-in overflow-y-auto rounded-card bg-sheet p-1 shadow-pop"
          >
            {options.map((option, index) => (
              <div
                key={option.value}
                id={optionId(index)}
                role="option"
                aria-selected={index === chosen}
                data-active={index === active || undefined}
                onPointerMove={() => setActive(index)}
                onClick={() => {
                  pick(index);
                  button.current?.focus();
                }}
                className={cn(
                  "flex w-full cursor-default items-center gap-2 rounded-control px-2 text-left text-ink data-active:bg-hover [&_svg]:size-(--icon-sm) [&_svg]:shrink-0",
                  option.description ? "min-h-[30px] py-1.5" : "h-[30px]",
                )}
              >
                {option.icon && <span className={cn("inline-flex shrink-0", option.description && "self-start pt-0.5")}>{option.icon}</span>}
                <span className="min-w-0 flex-1">
                  <span className="block truncate">
                    {option.label}
                    {option.hint && <span className="text-ink-3"> · {option.hint}</span>}
                  </span>
                  {option.description && <span className="block text-meta whitespace-normal text-ink-3">{option.description}</span>}
                </span>
                <span className="grid w-4 shrink-0 place-items-center">{index === chosen && <Check aria-hidden className="text-brand" />}</span>
              </div>
            ))}
          </div>,
          layer(),
        )}
    </>
  );
}
