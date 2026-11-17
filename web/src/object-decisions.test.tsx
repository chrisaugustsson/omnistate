// @vitest-environment jsdom
// The Decisions of a track and of a feature (UI redesign, slice S3;
// prototype tasks/ui-redesign/v2/track-decisions.html,
// feature-analyze-decisions.html): the decisions nobody replaced, newest first,
// a replaced one folded under what replaced it; a track's log says what each
// decision is about, a feature's which track recorded it. Only the track's
// owner records a decision (US-324), about the track, one of its features,
// its part or the project.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { API, BOB, FakeWebSocket, PROJECT_ID, PROJECT_PATH, WISION, fakeTrackApi, featureBody, track } from "./testing/track-api";

const TRACK_ID = "0192d7a0-4444-7000-8000-000000000001";
const OLD_TRACK = "0192d7a0-4444-7000-8000-000000000002";
const ANALYZE = "0192d7a0-5555-7000-8000-000000000001";
const TRACK_PATH = `${PROJECT_PATH}/tracks/${TRACK_ID}`;
const FEATURE_PATH = `${PROJECT_PATH}/parts/${WISION}/features/${ANALYZE}`;

const id = (n: number) => `0192d7a0-7777-7000-8000-${n.toString(16).padStart(12, "0")}`;

function decision(
  n: number,
  text: string,
  fields: { at: string; track?: string; about?: "track" | "feature" | "part" | "project"; replaces?: number; replacedBy?: number },
) {
  const about = fields.about ?? "track";
  return {
    id: id(n),
    project_id: PROJECT_ID,
    track_id: fields.track ?? TRACK_ID,
    about,
    feature_id: about === "feature" ? ANALYZE : null,
    feature_name: about === "feature" ? "Analyze" : null,
    part_id: about === "part" ? WISION : null,
    part_name: about === "part" ? "Wision" : null,
    question: `Question ${n}?`,
    decision: text,
    why: `Why ${n}`,
    author: { kind: "agent", role: "grill" },
    replaces_decision_id: fields.replaces ? id(fields.replaces) : null,
    replaced_by_decision_id: fields.replacedBy ? id(fields.replacedBy) : null,
    created_at: fields.at,
  };
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** The log's top rows: each decision as its first line. */
async function topDecisions() {
  const log = await screen.findByRole("list", { name: "Decisions" });
  return Array.from(log.querySelectorAll(":scope > li")).map((row) => row.querySelector("p")?.textContent);
}

const row = (text: string) => screen.getByText(text).closest("li") as HTMLElement;

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("a track's Decisions", () => {
  test("lists the current decisions newest first, each saying what it is about, with a replaced one folded under its replacer", async () => {
    fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows")],
      trackDecisions: {
        [TRACK_ID]: [
          decision(4, "Presets only", { at: "2026-11-14T08:00:00Z", about: "feature", replaces: 1 }),
          decision(3, "Through api-gateway", { at: "2026-11-12T08:00:00Z", about: "project" }),
          decision(2, "Wision draws on canvas", { at: "2026-11-11T08:00:00Z", about: "part" }),
          decision(1, "Presets plus a custom range", { at: "2026-11-10T08:00:00Z", about: "feature", replacedBy: 4 }),
        ],
      },
    });
    renderAt(`${TRACK_PATH}/decisions`);
    await waitFor(async () => expect(await topDecisions()).toEqual(["Presets only", "Through api-gateway", "Wision draws on canvas"]));
    expect(screen.getByText(/recorded on this track/).closest("p")?.textContent).toBe(
      "4 recorded on this track · 3 current · 1 replaced, folded under what replaced it",
    );
    expect(row("Presets only").textContent).toContain("about Analyze");
    expect(row("Through api-gateway").textContent).toContain("about the project");
    expect(row("Wision draws on canvas").textContent).toContain("about Wision");

    const fold = within(row("Presets only")).getByRole("button", { name: /Replaces/ });
    fireEvent.click(fold);
    const replaced = within(row("Presets only")).getByRole("list", { name: "Replaced decisions" });
    expect(within(replaced).getByText("Presets plus a custom range")).toBeTruthy();
  });

  test("the owner records a decision about one of the track's features, and it shows at once", async () => {
    const api = fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows")],
      trackFeatures: { [TRACK_ID]: [featureBody(ANALYZE, "Analyze")] },
      trackDecisions: { [TRACK_ID]: [] },
    });
    renderAt(`${TRACK_PATH}/decisions`);
    fireEvent.click(await screen.findByRole("button", { name: "Record decision" }));
    const form = await screen.findByRole("form", { name: "Record decision" });
    const about = within(form).getByLabelText("About") as HTMLSelectElement;
    await waitFor(() => expect(Array.from(about.options).map((option) => option.textContent)).toEqual(["This track", "Feature: Analyze", "Part: Wision", "The whole project"]));
    const record = within(form).getByRole("button", { name: "Record decision" }) as HTMLButtonElement;
    fireEvent.change(within(form).getByLabelText("Question"), { target: { value: "Which windows?" } });
    fireEvent.change(within(form).getByLabelText("Decision"), { target: { value: "Presets only" } });
    expect(record.disabled, "why is required").toBe(true);
    fireEvent.change(within(form).getByLabelText("Why"), { target: { value: "Custom ranges hid the switch" } });
    fireEvent.change(about, { target: { value: `feature:${ANALYZE}` } });
    fireEvent.click(record);

    await waitFor(async () => expect(await topDecisions()).toEqual(["Presets only"]));
    expect(api.calls.filter((call) => call.method === "POST").map(({ path, body }) => ({ path, body }))).toEqual([
      { path: `${API}/tracks/${TRACK_ID}/decisions`, body: { question: "Which windows?", decision: "Presets only", why: "Custom ranges hid the switch", feature_id: ANALYZE } },
    ]);
    expect(row("Presets only").textContent).toContain("Alice · Answers “Which windows?” · about Analyze");
  });

  test("anyone but the owner, an admin too, sees Record decision off, with who records them", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows", { owner_user_id: BOB })], trackDecisions: { [TRACK_ID]: [] } });
    renderAt(`${TRACK_PATH}/decisions`);
    const record = (await screen.findByRole("button", { name: "Record decision" })) as HTMLButtonElement;
    await waitFor(() => expect(record.title).toBe("Only Bob records decisions on this track"));
    expect(record.disabled).toBe(true);
  });
});

describe("a feature's Decisions", () => {
  test("lists the current decisions about it newest first, each linking to the track that recorded it, a replaced one folded under its replacer", async () => {
    fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows"), track(OLD_TRACK, "First chart", { state: "done" })],
      features: [featureBody(ANALYZE, "Analyze")],
      featureTracks: { [ANALYZE]: [TRACK_ID, OLD_TRACK] },
      featureDecisions: {
        [ANALYZE]: {
          decisions: [
            decision(4, "Presets only", { at: "2026-11-14T08:00:00Z", about: "feature", replaces: 1 }),
            decision(2, "At most 4 devices", { at: "2026-03-02T08:00:00Z", about: "feature", track: OLD_TRACK }),
            decision(1, "Presets plus a custom range", { at: "2026-03-01T08:00:00Z", about: "feature", track: OLD_TRACK, replacedBy: 4 }),
          ],
          earlier: [],
        },
      },
    });
    renderAt(`${FEATURE_PATH}/decisions`);
    await waitFor(async () => expect(await topDecisions()).toEqual(["Presets only", "At most 4 devices"]));
    expect(within(row("Presets only")).getByRole("link", { name: "Long windows" }).getAttribute("href")).toBe(TRACK_PATH);
    expect(within(row("At most 4 devices")).getByRole("link", { name: "First chart" }).getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${OLD_TRACK}`);
    const log = screen.getByRole("region", { name: "Decisions" });
    expect(within(log).getByText(/current/).closest("p")?.textContent).toBe("2 current · 1 replaced, folded under what replaced it");
  });
});
