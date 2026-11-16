import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Pencil, Plus, Trash2 } from "lucide-react";
import type { FeatureBody } from "@/api/generated";
import { ApiError } from "@/api/client";
import { createFeature, deleteFeature, renameFeature } from "@/api/features";
import { Empty, List, rowClass } from "@/components/project/items";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useFeatures } from "@/lib/use-features";

// The features of a part (D2): its capabilities, each with a name and a
// description, and a way to create, rename and delete one. A feature's
// description is set when it is created and not edited, as in the CLI.

type Params = { workspaceId: string; projectId: string; partId: string };

function messageOf(failure: unknown, what: string): string {
  if (failure instanceof ApiError && failure.status === 409 && failure.code === "name_taken")
    return "Another feature of the part has this name";
  if (failure instanceof ApiError && failure.status === 400) return "The name can not be blank";
  return `The feature could not be ${what}. Try again.`;
}

function CreateForm({ workspaceId, projectId, partId, onDone }: Params & { onDone: () => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const canSave = name.trim() !== "" && !saving;

  async function save() {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      await createFeature(workspaceId, projectId, partId, { name: name.trim(), description: description.trim() });
      onDone();
    } catch (failure) {
      setSaving(false);
      setError(messageOf(failure, "created"));
    }
  }

  return (
    <form
      className="space-y-3 rounded-xl border bg-card p-4"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <Input
        autoFocus
        aria-label="Feature name"
        placeholder="Name"
        value={name}
        className="max-w-xs"
        onChange={(event) => {
          setName(event.target.value);
          setError(null);
        }}
      />
      <Input
        aria-label="Feature description"
        placeholder="Description (optional)"
        value={description}
        onChange={(event) => setDescription(event.target.value)}
      />
      {error && (
        <p role="alert" className="text-[13px] text-destructive">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={!canSave}>
          Create feature
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function FeatureRow({ feature, params, onChanged }: { feature: FeatureBody; params: Params; onChanged: () => void }) {
  const [mode, setMode] = useState<"view" | "rename" | "delete">("view");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function close() {
    setMode("view");
    setError(null);
    setBusy(false);
  }

  async function run(action: () => Promise<unknown>, what: string) {
    setBusy(true);
    setError(null);
    try {
      await action();
      close();
      onChanged();
    } catch (failure) {
      setBusy(false);
      setError(messageOf(failure, what));
    }
  }

  if (mode === "rename") {
    const trimmed = text.trim();
    return (
      <li className="space-y-2 px-4 py-3">
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (trimmed && trimmed !== feature.name && !busy)
              void run(
                () => renameFeature(params.workspaceId, params.projectId, params.partId, feature.id, trimmed),
                "renamed",
              );
          }}
        >
          <Input
            autoFocus
            aria-label="Feature name"
            value={text}
            className="max-w-xs"
            onChange={(event) => {
              setText(event.target.value);
              setError(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") close();
            }}
          />
          <Button type="submit" size="sm" disabled={!trimmed || trimmed === feature.name || busy}>
            Save
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={close}>
            Cancel
          </Button>
        </form>
        {error && (
          <p role="alert" className="text-[13px] text-destructive">
            {error}
          </p>
        )}
      </li>
    );
  }

  if (mode === "delete") {
    return (
      <li className="space-y-2 px-4 py-3 text-sm">
        <div className="flex flex-wrap items-center gap-2">
          <span className="min-w-0 flex-1">
            Delete <span className="font-medium">{feature.name}</span>? The feature is hidden.
          </span>
          <Button
            type="button"
            variant="destructive"
            size="sm"
            disabled={busy}
            onClick={() =>
              void run(() => deleteFeature(params.workspaceId, params.projectId, params.partId, feature.id), "deleted")
            }
          >
            Delete
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={close}>
            Cancel
          </Button>
        </div>
        {error && (
          <p role="alert" className="text-[13px] text-destructive">
            {error}
          </p>
        )}
      </li>
    );
  }

  return (
    <li className="flex items-center transition-colors hover:bg-accent/60">
      <Link
        to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId"
        params={{ ...params, featureId: feature.id }}
        className={`${rowClass} min-w-0 flex-1 hover:bg-transparent`}
      >
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">{feature.name}</span>
          {feature.description && (
            <span className="mt-0.5 block truncate text-[13px] text-muted-foreground">
              {feature.description.split(/\r?\n/, 1)[0]}
            </span>
          )}
        </span>
      </Link>
      <div className="flex shrink-0 gap-1 pr-3">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => {
            setText(feature.name);
            setMode("rename");
          }}
        >
          <Pencil aria-hidden className="size-3.5" />
          <span className="sr-only">Rename {feature.name}</span>
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={() => setMode("delete")}>
          <Trash2 aria-hidden className="size-3.5" />
          <span className="sr-only">Delete {feature.name}</span>
        </Button>
      </div>
    </li>
  );
}

/** The part's features with a form to create one. */
export function FeaturesSection({ workspaceId, projectId, partId }: Params) {
  const params = { workspaceId, projectId, partId };
  const { features, failed, reload } = useFeatures(workspaceId, projectId, partId);
  const [creating, setCreating] = useState(false);

  return (
    <section aria-labelledby="features-title" className="space-y-3">
      <div className="flex items-center gap-2.5">
        <h2 id="features-title" className="text-[15px] font-semibold tracking-tight">
          Features
        </h2>
        <span className="text-sm text-muted-foreground tabular-nums">{features?.length ?? 0}</span>
        {!creating && (
          <Button type="button" variant="outline" size="sm" className="ml-auto" onClick={() => setCreating(true)}>
            <Plus aria-hidden className="size-3.5" />
            New feature
          </Button>
        )}
      </div>
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The features could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {creating && (
        <CreateForm
          {...params}
          onDone={() => {
            setCreating(false);
            reload();
          }}
        />
      )}
      {features &&
        (features.length === 0 ? (
          <Empty>No features yet</Empty>
        ) : (
          <List>
            {features.map((feature) => (
              <FeatureRow key={feature.id} feature={feature} params={params} onChanged={reload} />
            ))}
          </List>
        ))}
    </section>
  );
}
