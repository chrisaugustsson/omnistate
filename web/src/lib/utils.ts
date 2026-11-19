import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// tailwind-merge knows Tailwind's own scales only. Without the theme's names
// it reads `text-meta` as a colour, and drops it when a `text-ink-3` follows,
// so the text falls back to the body size; it also keeps two radii or shadows
// side by side. The names are those of the @theme in styles.css.
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      text: ["meta", "body", "em", "section", "title", "greeting"],
      radius: ["badge", "control", "card", "sheet"],
      shadow: ["control", "card", "sheet", "pop", "focus", "panel"],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
