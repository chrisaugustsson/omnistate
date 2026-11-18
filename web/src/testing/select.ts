import { fireEvent, screen, within } from "@testing-library/react";

// Driving the app's Select (components/shell/select.tsx) in tests: a combobox
// button whose options open in a listbox drawn on the page.

/** The names of the options `combobox` offers; leaves the list closed. */
export function optionNames(combobox: HTMLElement): string[] {
  fireEvent.click(combobox);
  const names = within(screen.getByRole("listbox")).getAllByRole("option").map((option) => option.textContent ?? "");
  fireEvent.click(combobox);
  return names;
}

/** Opens `combobox` and picks the option named `name`. */
export function choose(combobox: HTMLElement, name: string | RegExp) {
  fireEvent.click(combobox);
  fireEvent.click(within(screen.getByRole("listbox")).getByRole("option", { name }));
}
