import { useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { Box, FileText, Palette, PenTool } from "lucide-react";
import { ApiError } from "@/api/client";
import { linkTrackFeature } from "@/api/features";
import { createTrack } from "@/api/parts";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { cn } from "@/lib/utils";
import { useFeature, type FeatureData } from "./feature-data";
import { Dialog, DialogError } from "@/components/shell/dialog";
import { Field, Input, Textarea } from "@/components/shell/field";
import { buttonClass } from "./ui";

// The pieces of a feature's pages: its mark, the "Start track" action, the
// line naming the tracks that change it now, and its docs' glyph.

/** A feature's mark beside its title: the cube in a hairline square. */
export function FeatureMark() {
  return (
    <span aria-hidden className="grid size-7 shrink-0 place-items-center rounded-control border border-line bg-sheet text-ink-2">
      <Box strokeWidth={2.25} className="size-(--icon)" />
    </span>
  );
}

/** The first sentence or line of a description. */
export function firstLine(text: string): string {
  const line = text.split(/\r?\n/, 1)[0]?.trim() ?? "";
  const sentence = line.match(/^.*?[.!?](\s|$)/)?.[0]?.trim();
  return sentence ?? line;
}

/** "Start track": a new track on the feature's part that changes the feature, and to its page. */
export function StartTrack({ data, className }: { data: FeatureData; className?: string }) {
  const { workspaceId, projectId, partId, featureId, feature } = data;
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [tried, setTried] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();
  const close = () => {
    setOpen(false);
    setTried(false);
    setError(null);
  };

  async function start() {
    setTried(true);
    if (!name.trim()) {
      requestAnimationFrame(() => document.querySelector<HTMLElement>('[role="dialog"] [aria-invalid="true"]')?.focus());
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const track = await createTrack(workspaceId, projectId, { name: name.trim(), description: description.trim(), part_id: partId });
      await linkTrackFeature(workspaceId, projectId, track.id, featureId);
      setOpen(false);
      await navigate({ to: "/w/$workspaceId/projects/$projectId/tracks/$trackId", params: { workspaceId, projectId, trackId: track.id } });
    } catch (failure) {
      setError(
        failure instanceof ApiError && failure.code === "name_taken"
          ? "Another track of the project has this name."
          : "The track could not be started. Try again.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <button type="button" className={buttonClass({ variant: "primary" }, className)} onClick={() => setOpen(true)}>
        Start track
      </button>
      {open && (
        <Dialog
          title={`Start a track on ${feature?.name ?? "the feature"}`}
          description="A track is one change to the feature: grilled, designed, built and merged. You own it; agents write into it with your key."
          onClose={close}
          onSubmit={() => {
            if (!busy) void start();
          }}
          footerStart={error && <DialogError>{error}</DialogError>}
          footer={
            <>
              <button type="button" className={buttonClass()} onClick={close}>
                Cancel
              </button>
              <button type="submit" className={buttonClass({ variant: "primary" })} disabled={busy}>
                Start track
              </button>
            </>
          }
        >
          <Field label="Name" error={tried && !name.trim() ? "Give the track a name" : undefined}>
            {(control) => <Input {...control} placeholder="What changes, in a few words" value={name} onChange={(event) => setName(event.target.value)} />}
          </Field>
          <Field label="What it changes" optional>
            {(control) => <Textarea {...control} minRows={2} placeholder="One sentence for the track's list" value={description} onChange={(event) => setDescription(event.target.value)} />}
          </Field>
        </Dialog>
      )}
    </>
  );
}

/** One line naming the tracks in flight that change the feature now, each a link to its track; nothing while none does. */
export function ChangingNow() {
  const { changing, workspaceId, projectId } = useFeature();
  if (changing.length === 0) return null;
  return (
    <p className="mt-4 text-ink-2">
      Being changed now by{" "}
      {changing.map(({ track }, at) => (
        <span key={track.id}>
          {at > 0 && (at === changing.length - 1 ? " and " : ", ")}
          <Link
            to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
            params={{ workspaceId, projectId, trackId: track.id }}
            className="font-medium text-ink underline decoration-line-strong underline-offset-3 transition-colors duration-(--t-fast) hover:decoration-ink"
          >
            <StateIcon state={track.state} label={stateLabel(track.state)} className="mr-1 inline-block align-[-2px]" />
            {track.name}
          </Link>
        </span>
      ))}
      . Not on main yet.
    </p>
  );
}

export function DocGlyph({ kind, className }: { kind: string; className?: string }) {
  const Icon = kind === "design" ? PenTool : kind === "design-system" ? Palette : FileText;
  return <Icon aria-hidden strokeWidth={2.25} className={cn("size-(--icon-sm) shrink-0 text-ink-2", className)} />;
}
