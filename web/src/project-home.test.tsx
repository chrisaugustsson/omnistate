// @vitest-environment jsdom
// The Overview of a project (/w/<ws>/projects/<p>) and of a part
// (.../parts/<part>), and the Docs tabs of both (build plan, slice S2;
// prototype tasks/ui-redesign/v2/project.html, part-wision.html, docs.html).
// An Overview is the README (navigation F): the name and the description as
// plain page text, editable in place, "Add a description" when empty
// (decision U8), and nothing else. A Docs section lists the scope's docs
// grouped by pillar beside the open doc; with none open the newest opens, and
// on the phone the list is the page and a doc opens alone. The workspace's
// Projects lists its projects.
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  API,
  FakeWebSocket,
  PROJECT_PATH,
  WORKSPACE_ID,
  decision,
  doc,
  fakeScopeApi,
  feature,
  part,
  renderAt,
  shownText,
  track,
} from "./scope-pages-fake";

const main = () => screen.findByRole("main");
/** The rows of a section's list, as the reader sees each. */
const rows = (container: HTMLElement) => within(container).getAllByRole("listitem").map(shownText);

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-11-17T12:00:00Z"));
  vi.stubGlobal("scrollTo", () => {});
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the project's Overview", () => {
  test("with no description it offers Add a description; the text saved is the intro, and Edit opens it again", async () => {
    const api = fakeScopeApi({ description: null });
    renderAt(PROJECT_PATH);
    fireEvent.click(await within(await main()).findByRole("button", { name: "Add a description" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Description" }), {
      target: { value: "Sensor monitoring for customers.\n\nAsk Sara about readings." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: API, body: { description: "Sensor monitoring for customers.\n\nAsk Sara about readings." } }]));
    const page = await main();
    expect(await within(page).findByText("Ask Sara about readings.")).toBeTruthy();
    expect(within(page).getByText("Sensor monitoring for customers.").tagName).toBe("P");
    expect(within(page).queryByRole("button", { name: "Add a description" })).toBeNull();

    fireEvent.click(within(page).getByRole("button", { name: "Edit the description" }));
    expect((screen.getByRole("textbox", { name: "Description" }) as HTMLTextAreaElement).value).toBe(
      "Sensor monitoring for customers.\n\nAsk Sara about readings.",
    );
  });

  test("is the README: the name and the description as page text, with no lists of tracks, decisions or docs", async () => {
    const wision = part("Wision", "app");
    const analyze = feature(wision.id, "Analyze");
    const windows = track("Long time windows", wision.id, "building", { total: 2 });
    fakeScopeApi({
      description: "Sensor monitoring for customers.",
      parts: [wision],
      features: [analyze],
      tracks: [windows],
      changes: { [analyze.id]: [windows.track.id] },
      docs: [doc("Architecture overview", "architecture")],
      decisions: [decision("Apps reach services only through api-gateway", windows.track.id, { project: true })],
    });
    renderAt(PROJECT_PATH);

    const page = await main();
    expect(await within(page).findByRole("heading", { level: 1, name: "Monitoring platform" })).toBeTruthy();
    expect(within(page).getByText("Sensor monitoring for customers.").tagName).toBe("P");
    await waitFor(() => expect(screen.getByRole("link", { name: /^Tracks/ }).textContent).toContain("1"));
    expect(within(page).queryAllByRole("list")).toEqual([]);
    expect(within(page).queryByText(/Long time windows|Architecture overview|api-gateway/)).toBeNull();
  });
});

describe("a part's Overview", () => {
  test("is the README: the part's name and description, with no lists of its features or tracks", async () => {
    const wision = part("Wision", "app", { description: "The web app customers log in to." });
    const analyze = feature(wision.id, "Analyze", "Time-series chart per channel");
    const windows = track("Long time windows", wision.id, "building", { total: 2 });
    fakeScopeApi({ parts: [wision], features: [analyze], tracks: [windows], changes: { [analyze.id]: [windows.track.id] } });
    renderAt(`${PROJECT_PATH}/parts/${wision.id}`);

    const page = await main();
    expect(await within(page).findByRole("heading", { level: 1, name: "Wision" })).toBeTruthy();
    expect(within(page).getByText("The web app customers log in to.")).toBeTruthy();
    await waitFor(() => expect(screen.getByRole("link", { name: /^Features/ }).textContent).toContain("1"));
    expect(within(page).queryAllByRole("list")).toEqual([]);
    expect(within(page).queryByText(/Analyze|Long time windows/)).toBeNull();
  });

  test("its intro is edited in place: Edit opens the text, Save sends the new description and shows it", async () => {
    const wision = part("Wision", "app", { description: "The web app customers log in to." });
    const api = fakeScopeApi({ parts: [wision] });
    renderAt(`${PROJECT_PATH}/parts/${wision.id}`);

    const page = await main();
    fireEvent.click(await within(page).findByRole("button", { name: "Edit the description" }));
    const text = screen.getByRole("textbox", { name: "Description" }) as HTMLTextAreaElement;
    expect(text.value).toBe("The web app customers log in to.");
    fireEvent.change(text, { target: { value: "Where customers watch their sensors." } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(api.writes).toEqual([{ method: "PATCH", path: `${API}/parts/${wision.id}`, body: { description: "Where customers watch their sensors." } }]),
    );
    expect(await within(page).findByText("Where customers watch their sensors.")).toBeTruthy();
    expect(within(page).queryByText("The web app customers log in to.")).toBeNull();
  });
});

describe("the Docs tabs", () => {
  test("the project's Docs opens its most recently updated doc beside its docs grouped by pillar, in pillar order", async () => {
    const wision = part("Wision", "app");
    const onboarding = doc("Onboarding", "spec", { updatedAt: "2026-11-12T08:00:00Z" });
    const overview = doc("Architecture overview", "architecture", { updatedAt: "2026-11-16T08:00:00Z" });
    const routes = doc("Gateway routes", "architecture", { updatedAt: "2026-11-10T08:00:00Z" });
    const tables = doc("Readings tables", "db", { updatedAt: "2026-11-11T08:00:00Z" });
    fakeScopeApi({ parts: [wision], docs: [tables, routes, overview, onboarding, doc("Wision tokens", "ui", { part: wision.id, updatedAt: "2026-11-17T08:00:00Z" })] });
    const router = renderAt(`${PROJECT_PATH}/docs`);

    await waitFor(() => expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/docs/${overview.id}`));
    const list = await screen.findByRole("navigation", { name: "Docs" });
    expect(within(list).getAllByRole("region").map((group) => [group.getAttribute("aria-label"), within(group).getAllByRole("link").map(shownText)])).toEqual([
      ["Spec", ["Onboarding"]],
      ["Architecture", ["Architecture overview", "Gateway routes"]],
      ["Database", ["Readings tables"]],
    ]);
    expect(within(list).getByRole("link", { name: "Architecture overview" }).getAttribute("aria-current")).toBe("page");
  });

  test("a part's Docs lists the docs of the part and of its features, not other parts', and the find field narrows them by title", async () => {
    const wision = part("Wision", "app");
    const iot = part("iot-service", "service");
    const analyze = feature(wision.id, "Analyze");
    const compare = doc("How compare works", "architecture", { feature: analyze.id });
    const system = doc("Design system", "ui", { part: wision.id });
    fakeScopeApi({ parts: [wision, iot], features: [analyze], docs: [compare, system, doc("Readings tables", "db", { part: iot.id })] });
    renderAt(`${PROJECT_PATH}/parts/${wision.id}/docs/${compare.id}`);

    const list = await screen.findByRole("navigation", { name: "Docs" });
    await within(list).findByText("How compare works");
    expect(within(list).getAllByRole("link").map((link) => [shownText(link), link.getAttribute("href")])).toEqual([
      ["Design system", `${PROJECT_PATH}/parts/${wision.id}/docs/${system.id}`],
      ["How compare works Analyze", `${PROJECT_PATH}/parts/${wision.id}/docs/${compare.id}`],
    ]);
    fireEvent.change(within(list).getByRole("searchbox", { name: "Find a doc" }), { target: { value: "COMPARE" } });
    expect(within(list).getAllByRole("link").map(shownText)).toEqual(["How compare works Analyze"]);
  });

  test("a part's Docs with no docs opens none and says the part has none yet", async () => {
    const wision = part("Wision", "app");
    const iot = part("iot-service", "service");
    fakeScopeApi({ parts: [wision, iot], docs: [doc("Design system", "ui", { part: wision.id })] });
    const router = renderAt(`${PROJECT_PATH}/parts/${iot.id}/docs`);
    expect(await screen.findByText(/No docs on iot-service yet/)).toBeTruthy();
    expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/parts/${iot.id}/docs`);
  });
});

describe("the Docs sections on the phone", () => {
  beforeEach(() => {
    vi.stubGlobal("matchMedia", (media: string) => ({ media, matches: true, addEventListener() {}, removeEventListener() {} }));
  });

  test("a project's Docs is the list, opening no doc; a doc opens alone, with All docs back to the list", async () => {
    const overview = doc("Architecture overview", "architecture", { updatedAt: "2026-11-16T08:00:00Z" });
    const onboarding = doc("Onboarding", "spec", { updatedAt: "2026-11-12T08:00:00Z" });
    fakeScopeApi({ docs: [overview, onboarding] });
    const router = renderAt(`${PROJECT_PATH}/docs`);

    const list = await screen.findByRole("navigation", { name: "Docs" });
    expect(within(list).getAllByRole("link").map(shownText)).toEqual(["Onboarding", "Architecture overview"]);
    expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/docs`);

    fireEvent.click(within(list).getByRole("link", { name: "Architecture overview" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Architecture overview" })).toBeTruthy();
    expect(screen.queryByRole("navigation", { name: "Docs" })).toBeNull();
    fireEvent.click(screen.getByRole("link", { name: "All docs" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/docs`));
  });
});

describe("the workspace's Projects", () => {
  test("a row per project shows its name, its parts, and how many tracks are in flight, and opens its Overview", async () => {
    const wision = part("Wision", "app");
    const iot = part("iot-service", "service");
    fakeScopeApi({
      parts: [wision, iot],
      tracks: [track("Long time windows", wision.id, "planning"), track("Aggregated readings", iot.id, "building"), track("First chart", wision.id, "done")],
    });
    renderAt(`/w/${WORKSPACE_ID}/projects`);

    const list = await within(await main()).findByRole("list", { name: "Projects" });
    await waitFor(() => expect(rows(list)).toEqual(["Monitoring platform 2 in flight"]));
    const row = within(list).getByRole("link");
    expect(row.getAttribute("href")).toBe(PROJECT_PATH);
    expect(within(row).getByRole("img", { name: "Wision, iot-service" })).toBeTruthy();
  });
});
