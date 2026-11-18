// @vitest-environment jsdom
// The app's dialog and select (components/shell/dialog.tsx, select.tsx), as
// every create form uses them: the first field takes the focus, Enter in a
// field submits, Esc closes and gives the focus back to what opened it, Tab
// stays inside; the select opens on a key, moves with the arrows and picks
// with Enter, and its Esc closes only the list.
import { useState } from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { Dialog } from "./components/shell/dialog";
import { Field, Input } from "./components/shell/field";
import { Select } from "./components/shell/select";

afterEach(cleanup);

function Harness({ onSubmit }: { onSubmit: (values: { name: string; kind: string }) => void }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [kind, setKind] = useState("");
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Create part
      </button>
      {open && (
        <Dialog
          title="Create a part"
          onClose={() => setOpen(false)}
          onSubmit={() => onSubmit({ name, kind })}
          footer={
            <>
              <button type="button" onClick={() => setOpen(false)}>
                Cancel
              </button>
              <button type="submit">Create part</button>
            </>
          }
        >
          <Field label="Name">{(control) => <Input {...control} value={name} onChange={(event) => setName(event.target.value)} />}</Field>
          <Field label="Kind">
            {(control) => (
              <Select
                {...control}
                value={kind}
                onChange={setKind}
                placeholder="App or service"
                options={[
                  { value: "app", label: "App" },
                  { value: "service", label: "Service" },
                ]}
              />
            )}
          </Field>
        </Dialog>
      )}
    </>
  );
}

function open() {
  const submit = vi.fn();
  render(<Harness onSubmit={submit} />);
  const opener = screen.getByRole("button", { name: "Create part" });
  opener.focus();
  fireEvent.click(opener);
  return { submit, opener, dialog: screen.getByRole("dialog", { name: "Create a part" }) };
}

describe("a dialog", () => {
  test("opens with the focus in its first field, and Enter there submits it with what was typed", () => {
    const { submit, dialog } = open();
    const name = within(dialog).getByRole("textbox", { name: "Name" });
    expect(document.activeElement).toBe(name);

    fireEvent.change(name, { target: { value: "alarm-service" } });
    fireEvent.submit(name.closest("form") as HTMLFormElement);
    expect(submit).toHaveBeenCalledWith({ name: "alarm-service", kind: "" });
  });

  test("Esc closes it and gives the focus back to the button that opened it", () => {
    const { opener, dialog } = open();
    fireEvent.keyDown(within(dialog).getByRole("textbox", { name: "Name" }), { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  test("Tab from its last control goes back to its first, and Shift+Tab from the first to the last", () => {
    const { dialog } = open();
    const close = within(dialog).getByRole("button", { name: "Close" });
    const create = within(dialog).getByRole("button", { name: "Create part" });
    create.focus();
    fireEvent.keyDown(create, { key: "Tab" });
    expect(document.activeElement).toBe(close);
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(create);
  });

  test("a click on the scrim closes it; a click inside does not", () => {
    const { dialog } = open();
    fireEvent.pointerDown(within(dialog).getByRole("textbox", { name: "Name" }));
    expect(screen.queryByRole("dialog")).not.toBeNull();
    fireEvent.pointerDown(dialog.parentElement as HTMLElement);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("a select in a dialog", () => {
  test("opens on ArrowDown, moves with the arrows, picks with Enter and shows the choice; the dialog stays open", () => {
    const { submit, dialog } = open();
    const kind = within(dialog).getByRole("combobox", { name: "Kind" });
    expect(kind.textContent).toBe("App or service");

    fireEvent.keyDown(kind, { key: "ArrowDown" });
    expect(kind.getAttribute("aria-expanded")).toBe("true");
    const options = within(screen.getByRole("listbox", { name: "Kind" })).getAllByRole("option");
    expect(kind.getAttribute("aria-activedescendant")).toBe(options[0].id);
    fireEvent.keyDown(kind, { key: "ArrowDown" });
    expect(kind.getAttribute("aria-activedescendant")).toBe(options[1].id);
    fireEvent.keyDown(kind, { key: "Enter" });

    expect(screen.queryByRole("listbox")).toBeNull();
    expect(kind.textContent).toBe("Service");
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(submit, "Enter on the select picks, it does not submit").not.toHaveBeenCalled();
  });

  test("Esc on an open select closes the list only; a second Esc closes the dialog", () => {
    const { dialog } = open();
    const kind = within(dialog).getByRole("combobox", { name: "Kind" });
    fireEvent.click(kind);
    expect(screen.getByRole("listbox")).toBeTruthy();

    fireEvent.keyDown(kind, { key: "Escape" });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("dialog")).toBeTruthy();

    fireEvent.keyDown(kind, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  test("a letter moves to the next option starting with it", () => {
    const { dialog } = open();
    const kind = within(dialog).getByRole("combobox", { name: "Kind" });
    fireEvent.click(kind);
    fireEvent.keyDown(kind, { key: "s" });
    fireEvent.keyDown(kind, { key: "Enter" });
    expect(kind.textContent).toBe("Service");
  });
});
