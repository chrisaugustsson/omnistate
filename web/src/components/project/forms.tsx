import { useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import type { PartBody } from "@/api/generated";
import { ApiError } from "@/api/client";
import { createFeature } from "@/api/features";
import { createPart, createTrack } from "@/api/parts";
import { cn } from "@/lib/utils";
import { btn, btnPrimary, field } from "./rows";

// The create forms of the project's and the part's pages: a part, a feature
// and a track. Each opens under the page's head from its button there, and
// closes when it saves or is cancelled.

type Params = { workspaceId: string; projectId: string };

/** A form in a quiet box under the head, with its fields, an error and Save/Cancel. */
function FormBox({
  label,
  onSubmit,
  children,
  error,
  submit,
  canSave,
  onCancel,
}: {
  label: string;
  onSubmit: () => void;
  children: ReactNode;
  error: string | null;
  submit: string;
  canSave: boolean;
  onCancel: () => void;
}) {
  return (
    <form
      aria-label={label}
      className="mt-4 max-w-(--main-max) space-y-2 rounded-card border border-line bg-frame p-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (canSave) onSubmit();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      {children}
      {error && (
        <p role="alert" className="text-danger-ink">
          {error}
        </p>
      )}
      <div className="flex gap-2 pt-1">
        <button type="submit" className={btnPrimary} disabled={!canSave}>
          {submit}
        </button>
        <button type="button" className={btn} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

function partMessage(failure: unknown): string {
  if (failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken") return "Another part of the project has this name";
  if (failure instanceof ApiError && failure.status === 400) return "The name can not be blank";
  return "The part could not be created. Try again.";
}

/** Creates a part: a name, a kind (app or service, no default) and an optional description. */
export function CreatePartForm({ workspaceId, projectId, onDone }: Params & { onDone: (created: boolean) => void }) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await createPart(workspaceId, projectId, { name: name.trim(), kind, description: description.trim() });
      onDone(true);
    } catch (failure) {
      setSaving(false);
      setError(partMessage(failure));
    }
  }

  return (
    <FormBox
      label="Create part"
      submit="Create part"
      canSave={name.trim() !== "" && kind !== "" && !saving}
      error={error}
      onSubmit={() => void save()}
      onCancel={() => onDone(false)}
    >
      <div className="flex flex-wrap gap-2">
        <input
          autoFocus
          aria-label="Part name"
          placeholder="Name"
          value={name}
          className={cn(field, "max-w-xs flex-1")}
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
        />
        <select aria-label="Part kind" value={kind} onChange={(event) => setKind(event.target.value)} className={cn(field, "w-36")}>
          <option value="" disabled>
            Kind
          </option>
          <option value="app">app</option>
          <option value="service">service</option>
        </select>
      </div>
      <input
        aria-label="Part description"
        placeholder="What it is (optional)"
        value={description}
        className={field}
        onChange={(event) => setDescription(event.target.value)}
      />
    </FormBox>
  );
}

function featureMessage(failure: unknown): string {
  if (failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken") return "Another feature of the part has this name";
  if (failure instanceof ApiError && failure.status === 400) return "The name can not be blank";
  return "The feature could not be created. Try again.";
}

/** Creates a feature of a part: a name and what it does. */
export function CreateFeatureForm({ workspaceId, projectId, partId, onDone }: Params & { partId: string; onDone: (created: boolean) => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await createFeature(workspaceId, projectId, partId, { name: name.trim(), description: description.trim() });
      onDone(true);
    } catch (failure) {
      setSaving(false);
      setError(featureMessage(failure));
    }
  }

  return (
    <FormBox
      label="Create feature"
      submit="Create feature"
      canSave={name.trim() !== "" && !saving}
      error={error}
      onSubmit={() => void save()}
      onCancel={() => onDone(false)}
    >
      <input
        autoFocus
        aria-label="Feature name"
        placeholder="Name"
        value={name}
        className={cn(field, "max-w-xs")}
        onChange={(event) => {
          setName(event.target.value);
          setError(null);
        }}
      />
      <input
        aria-label="Feature description"
        placeholder="What it does (optional)"
        value={description}
        className={field}
        onChange={(event) => setDescription(event.target.value)}
      />
    </FormBox>
  );
}

/**
 * Starts a track: a name, an optional description, and its part. With
 * `fixedPartId` the track goes on that part; otherwise the form asks for one,
 * starting empty. A project with no parts can not start one: the form says so
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
  const [error, setError] = useState<string | null>(null);
  const part = fixedPartId ?? partId;

  if (!fixedPartId && parts.length === 0) {
    return (
      <div role="status" className="mt-4 max-w-(--main-max) rounded-card border border-line bg-frame p-3 text-ink-2">
        A track goes on a part, and the project has none yet.{" "}
        <Link to="/w/$workspaceId/projects/$projectId/parts" params={{ workspaceId, projectId }} className="font-medium text-ink underline underline-offset-2">
          Create a part
        </Link>{" "}
        first.
      </div>
    );
  }

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await createTrack(workspaceId, projectId, { name: name.trim(), description: description.trim(), part_id: part });
      onDone(true);
    } catch (failure) {
      setSaving(false);
      setError(
        failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken"
          ? "Another track of the project has this name"
          : "The track could not be created. Try again.",
      );
    }
  }

  return (
    <FormBox
      label="Start track"
      submit="Start track"
      canSave={name.trim() !== "" && part !== "" && !saving}
      error={error}
      onSubmit={() => void save()}
      onCancel={() => onDone(false)}
    >
      <div className="flex flex-wrap gap-2">
        <input
          autoFocus
          aria-label="Track name"
          placeholder="Name"
          value={name}
          className={cn(field, "max-w-xs flex-1")}
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
        />
        {!fixedPartId && (
          <select aria-label="Part" value={partId} onChange={(event) => setPartId(event.target.value)} className={cn(field, "w-44")}>
            <option value="" disabled>
              Part
            </option>
            {parts.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name}
              </option>
            ))}
          </select>
        )}
      </div>
      <input
        aria-label="Track description"
        placeholder="What it changes (optional)"
        value={description}
        className={field}
        onChange={(event) => setDescription(event.target.value)}
      />
    </FormBox>
  );
}
