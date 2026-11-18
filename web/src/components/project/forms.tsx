import { useState } from "react";
import { Link } from "@tanstack/react-router";
import type { PartBody } from "@/api/generated";
import { ApiError } from "@/api/client";
import { createFeature } from "@/api/features";
import { createPart, createTrack } from "@/api/parts";
import { Dialog, DialogError } from "@/components/shell/dialog";
import { Field, Input } from "@/components/shell/field";
import { PartIcon } from "@/components/shell/part-icon";
import { partTone } from "@/lib/identity";
import { Select } from "@/components/shell/select";
import { btn, btnPrimary } from "./rows";

// The create dialogs of the project's and the part's pages: a part, a feature
// and a track. Each opens from its button in the page's head and closes when
// it saves or is cancelled. A required field left empty is said under it when
// the dialog is submitted; the server's answer about a name goes under the name.

type Params = { workspaceId: string; projectId: string };

/** Cancel and the one primary, which is off only while saving. */
function Actions({ submit, saving, onCancel }: { submit: string; saving: boolean; onCancel: () => void }) {
  return (
    <>
      <button type="button" className={btn} onClick={onCancel}>
        Cancel
      </button>
      <button type="submit" className={btnPrimary} disabled={saving}>
        {submit}
      </button>
    </>
  );
}

/** Puts the focus on the first field the dialog marked invalid. */
function focusInvalid() {
  requestAnimationFrame(() => document.querySelector<HTMLElement>('[role="dialog"] [aria-invalid="true"]')?.focus());
}

type NameError = { field: "name"; text: string } | { field: "form"; text: string };

function partMessage(failure: unknown): NameError {
  if (failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken") return { field: "name", text: "Another part of the project has this name" };
  if (failure instanceof ApiError && failure.status === 400) return { field: "name", text: "The name can not be blank" };
  return { field: "form", text: "The part could not be created. Try again." };
}

const KINDS = [
  { value: "app", label: "App", description: "Has screens people use. Its tracks get a Design step." },
  { value: "service", label: "Service", description: "No screens: an API, a worker, a pipeline. Its tracks skip Design." },
] as const;

/** Creates a part: a name, a kind (app or service, no default) and an optional line on what it does. */
export function CreatePartForm({ workspaceId, projectId, onDone }: Params & { onDone: (created: boolean) => void }) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<"" | "app" | "service">("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);
  const [tried, setTried] = useState(false);
  const [error, setError] = useState<NameError | null>(null);
  const nameMissing = tried && name.trim() === "";
  const kindMissing = tried && kind === "";

  async function save() {
    setTried(true);
    if (name.trim() === "" || kind === "") return focusInvalid();
    setSaving(true);
    setError(null);
    try {
      await createPart(workspaceId, projectId, { name: name.trim(), kind, description: description.trim() });
      onDone(true);
    } catch (failure) {
      setSaving(false);
      setError(partMessage(failure));
      focusInvalid();
    }
  }

  return (
    <Dialog
      wide
      title="Create a part"
      description="One app or one service of the project. It starts with no features: add them on its page, or let a track add them."
      onClose={() => onDone(false)}
      onSubmit={() => void save()}
      footerStart={error?.field === "form" && <DialogError>{error.text}</DialogError>}
      footer={<Actions submit="Create part" saving={saving} onCancel={() => onDone(false)} />}
    >
      <Field label="Name" error={nameMissing ? "Give the part a name" : error?.field === "name" ? error.text : undefined}>
        {(control) => (
          <Input
            {...control}
            placeholder="alarm-service"
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              setError(null);
            }}
          />
        )}
      </Field>
      <Field label="Kind" error={kindMissing ? "Choose a kind" : undefined}>
        {(control) => (
          <Select
            {...control}
            value={kind}
            onChange={setKind}
            placeholder="App or service"
            options={KINDS}
          />
        )}
      </Field>
      <Field label="What it does" optional hint="Shown next to its name on the project's pages.">
        {(control) => (
          <Input {...control} placeholder="Evaluates alarm thresholds and sends notifications" value={description} onChange={(event) => setDescription(event.target.value)} />
        )}
      </Field>
    </Dialog>
  );
}

function featureMessage(failure: unknown): NameError {
  if (failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken") return { field: "name", text: "Another feature of the part has this name" };
  if (failure instanceof ApiError && failure.status === 400) return { field: "name", text: "The name can not be blank" };
  return { field: "form", text: "The feature could not be created. Try again." };
}

/** Creates a feature of a part: a name and what it does. */
export function CreateFeatureForm({
  workspaceId,
  projectId,
  partId,
  partName,
  onDone,
}: Params & { partId: string; partName?: string; onDone: (created: boolean) => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);
  const [tried, setTried] = useState(false);
  const [error, setError] = useState<NameError | null>(null);

  async function save() {
    setTried(true);
    if (name.trim() === "") return focusInvalid();
    setSaving(true);
    setError(null);
    try {
      await createFeature(workspaceId, projectId, partId, { name: name.trim(), description: description.trim() });
      onDone(true);
    } catch (failure) {
      setSaving(false);
      setError(featureMessage(failure));
      focusInvalid();
    }
  }

  return (
    <Dialog
      title="Create a feature"
      description={`Something ${partName ?? "the part"} does for the people using it. Tracks change it; its decisions and docs gather on its page.`}
      onClose={() => onDone(false)}
      onSubmit={() => void save()}
      footerStart={error?.field === "form" && <DialogError>{error.text}</DialogError>}
      footer={<Actions submit="Create feature" saving={saving} onCancel={() => onDone(false)} />}
    >
      <Field label="Name" error={tried && name.trim() === "" ? "Give the feature a name" : error?.field === "name" ? error.text : undefined}>
        {(control) => (
          <Input
            {...control}
            placeholder="Alarms"
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              setError(null);
            }}
          />
        )}
      </Field>
      <Field label="What it does" optional>
        {(control) => <Input {...control} placeholder="Thresholds per channel, notify by email" value={description} onChange={(event) => setDescription(event.target.value)} />}
      </Field>
    </Dialog>
  );
}

/**
 * Starts a track: its part, a name and an optional description. With
 * `fixedPartId` the track goes on that part; otherwise the dialog asks for one,
 * starting empty. A project with no parts can not start one: the dialog says so
 * and points to Parts.
 */
export function StartTrackForm({
  workspaceId,
  projectId,
  fixedPartId,
  parts,
  onDone,
}: Params & { fixedPartId?: string; parts: readonly PartBody[]; onDone: (created: boolean) => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [partId, setPartId] = useState("");
  const [saving, setSaving] = useState(false);
  const [tried, setTried] = useState(false);
  const [error, setError] = useState<NameError | null>(null);
  const part = fixedPartId ?? partId;
  const close = () => onDone(false);

  if (!fixedPartId && parts.length === 0) {
    return (
      <Dialog
        title="Start a track"
        onClose={close}
        footer={
          <button type="button" className={btn} onClick={close}>
            Close
          </button>
        }
      >
        <p role="status" className="text-ink-2">
          A track goes on a part, and the project has none yet.{" "}
          <Link to="/w/$workspaceId/projects/$projectId/parts" params={{ workspaceId, projectId }} className="font-medium text-brand-ink underline-offset-2 hover:underline">
            Create a part
          </Link>{" "}
          first.
        </p>
      </Dialog>
    );
  }

  async function save() {
    setTried(true);
    if (name.trim() === "" || part === "") return focusInvalid();
    setSaving(true);
    setError(null);
    try {
      await createTrack(workspaceId, projectId, { name: name.trim(), description: description.trim(), part_id: part });
      onDone(true);
    } catch (failure) {
      setSaving(false);
      setError(
        failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken"
          ? { field: "name", text: "Another track of the project has this name" }
          : { field: "form", text: "The track could not be created. Try again." },
      );
      focusInvalid();
    }
  }

  const fixed = parts.find((one) => one.id === fixedPartId);
  return (
    <Dialog
      title={fixed ? `Start a track on ${fixed.name}` : "Start a track"}
      description="One piece of work on one part. You own it; agents write into it with your key."
      onClose={close}
      onSubmit={() => void save()}
      footerStart={error?.field === "form" && <DialogError>{error.text}</DialogError>}
      footer={<Actions submit="Start track" saving={saving} onCancel={close} />}
    >
      {!fixedPartId && (
        <Field
          label="Part"
          hint="An app's tracks get a Design step; a service's do not. Work across parts becomes one track per part."
          error={tried && partId === "" ? "Choose the part it changes" : undefined}
        >
          {(control) => (
            <Select
              {...control}
              value={partId}
              onChange={setPartId}
              placeholder="Choose a part"
              options={parts.map((one) => ({ value: one.id, label: one.name, hint: one.kind, icon: <PartIcon name={one.name} kind={one.kind} tone={partTone(parts, one.id)} size="sm" /> }))}
            />
          )}
        </Field>
      )}
      <Field label="Name" error={tried && name.trim() === "" ? "Give the track a name" : error?.field === "name" ? error.text : undefined}>
        {(control) => (
          <Input
            {...control}
            placeholder="What changes, in a few words"
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              setError(null);
            }}
          />
        )}
      </Field>
      <Field label="What it changes" optional>
        {(control) => <Input {...control} placeholder="One sentence for the track's list" value={description} onChange={(event) => setDescription(event.target.value)} />}
      </Field>
    </Dialog>
  );
}
