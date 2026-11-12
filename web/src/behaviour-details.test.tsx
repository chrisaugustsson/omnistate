// @vitest-environment jsdom
// US-227: the details part reads the chosen node in full from
// GET .../behaviour/nodes/<node>: its text, its questions with every answer and
// who gave it, and the rules that reach it. The browser's fetch is replaced by
// a fake of that one request.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import type { TreeNode } from "@/api/behaviour";
import { Details, who } from "@/components/behaviour/details";
import { Text } from "@/components/behaviour/text";

const WORKSPACE_ID = "w1";
const PROJECT_ID = "p1";
const FEATURE_ID = "f1";
const NODE_PATH = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/features/${FEATURE_ID}/behaviour/nodes/`;
const ME = "user-me";
const OTHER = "user-other";
const TODAY = "2026-11-12T09:00:00Z";
const LONG_AGO = "2025-03-04T09:00:00Z";

const AGENT = { kind: "agent", run: "r7", role: "grill", model: "m", harness: "h" };
const PERSON_ME = { kind: "human", user_id: ME };
const PERSON_OTHER = { kind: "human", user_id: OTHER };

/** A node of the list GET .../behaviour sends. */
function listed(id: string, parentId: string | null, level: number, title: string, fields: Partial<TreeNode> = {}): TreeNode {
  return {
    id,
    parent_id: parentId,
    path: id,
    slug: null,
    position: 0,
    level,
    title,
    summary: "",
    status: "answered",
    version: 1,
    open_questions: 0,
    default_answers: 0,
    needs_pass: false,
    same_as: null,
    handoff: null,
    ...fields,
  } as TreeNode;
}

const TREE = [
  listed("feature", null, 0, "Disc golf putting trainer"),
  listed("story", "feature", 1, "As a player, I want to add a friend, so that we can practise together"),
  listed("search", "story", 2, "Player searches for a player"),
  listed("none", "search", 3, "No player is found"),
];

function answer(text: string, author: Record<string, unknown>, givenBy: string, at = LONG_AGO) {
  return { answer: text, at, author, given_by: givenBy };
}

function question(id: string, text: string, fields: { state?: string; asked_by?: Record<string, unknown>; answers?: ReturnType<typeof answer>[] }) {
  const answers = fields.answers ?? [];
  return {
    id,
    node_id: "none",
    node_path: "none",
    question: text,
    state: fields.state ?? "answered",
    asked_by: fields.asked_by ?? AGENT,
    answer: answers.at(-1) ?? null,
    answers,
    created_at: LONG_AGO,
    version: 1,
  };
}

function rule(id: string, nodeId: string, scope: string, text: string) {
  return { id, node_id: nodeId, node_path: nodeId, scope, text };
}

/** GET .../behaviour/nodes/none answers with these questions and rules, or with `status` when given. */
function fakeNode(read: { questions?: unknown[]; rules?: unknown[]; status?: number }) {
  const body = {
    node: {
      id: "none",
      parent_id: "search",
      path: "none",
      position: 0,
      level: 3,
      title: "No player is found",
      summary: "",
      status: "answered",
      situation: "",
      behaviour: "",
      needs_pass: false,
      created_at: LONG_AGO,
      created_by: AGENT,
      updated_at: LONG_AGO,
      version: 1,
    },
    chain: [],
    children: [],
    siblings: [],
    questions: read.questions ?? [],
    rules: read.rules ?? [],
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (url !== `${NODE_PATH}none`) return new Response("{}", { status: 404 });
      if (read.status) return new Response(JSON.stringify({ error: "internal" }), { status: read.status, headers: { "content-type": "application/json" } });
      return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
    }),
  );
}

function renderChosen(node: TreeNode = TREE[3], tree: TreeNode[] = TREE, onChoose: (id: string) => void = () => {}) {
  render(<Details node={node} tree={tree} me={ME} onChoose={onChoose} workspaceId={WORKSPACE_ID} projectId={PROJECT_ID} featureId={FEATURE_ID} />);
  return screen.getByRole("region", { name: "Details" });
}

const questionEl = (details: HTMLElement, id: string) => details.querySelector<HTMLElement>(`[data-question="${id}"]`)!;

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("who gave an answer", () => {
  test("is You, A member, the agent nobody was asked, or a person written down by the agent, and never a name", () => {
    expect(who(answer("a", PERSON_ME, "human"), ME)).toBe("You");
    expect(who(answer("a", PERSON_OTHER, "human"), ME)).toBe("A member");
    expect(who(answer("a", AGENT, "agent"), ME)).toBe("Agent r7/grill, nobody was asked");
    expect(who(answer("a", AGENT, "human"), ME)).toBe("A person, written down by agent r7/grill");
  });
});

describe("the details of a chosen node", () => {
  test("says 1 rule from above also holds here when only one does", async () => {
    fakeNode({ rules: [rule("r-feature", "feature", "feature", "Works offline")] });
    const details = renderChosen();
    await waitFor(() => expect(details.querySelector("details summary")).not.toBeNull());

    expect(details.querySelector("details summary")!.textContent).toBe("1 rule from above also holds here");
  });

  test("lists its own rules by how far they reach and opens the rules from above by where they were written", async () => {
    fakeNode({
      rules: [
        rule("r-node", "none", "node", "Say that nobody was found"),
        rule("r-story-wide", "none", "story", "Keep the search box filled"),
        rule("r-feature-wide", "none", "feature", "Never show an email address"),
        rule("r-parent", "search", "node", "Search by player name (default)"),
        rule("r-story", "story", "story", "Friends are two-way"),
        rule("r-feature", "feature", "feature", "Works offline"),
      ],
    });
    const details = renderChosen();
    await waitFor(() => expect(within(details).getByText("Say that nobody was found")).toBeTruthy());

    expect(within(details).getByText("Say that nobody was found").previousElementSibling?.textContent).toBe("This node");
    expect(within(details).getByText("Keep the search box filled").previousElementSibling?.textContent).toBe("This story");
    expect(within(details).getByText("Never show an email address").previousElementSibling?.textContent).toBe("Whole feature");
    const above = details.querySelector("details")!;
    expect(above.querySelector("summary")!.textContent).toBe("3 rules from above also hold here");
    expect(within(above).getByText("Search by player name (default)").previousElementSibling?.textContent).toBe("Player searches for a player");
    expect(within(above).getByText("Friends are two-way").previousElementSibling?.textContent).toBe("Story");
    expect(within(above).getByText("Works offline").previousElementSibling?.textContent).toBe("Feature");
    expect(within(above).queryByText("Say that nobody was found")).toBeNull();
    expect(within(details).getAllByText("Works offline")).toHaveLength(1);
  });

  test("lists waiting questions, then remarks, then answers by default, then the rest, and counts only waiting ones as open", async () => {
    fakeNode({
      questions: [
        question("q-rest", "Who sees the search?", { answers: [answer("Everyone", PERSON_OTHER, "human")] }),
        question("q-default", "How many results show?", { answers: [answer("Ten", AGENT, "agent")] }),
        question("q-remark", "Check the wording here", { state: "open", asked_by: PERSON_ME }),
        question("q-waiting", "What if the name has a typo?", { state: "open", asked_by: AGENT }),
      ],
    });
    const details = renderChosen();
    await waitFor(() => expect(details.querySelectorAll("[data-question]")).toHaveLength(4));

    const order = [...details.querySelectorAll("[data-question]")].map((one) => one.getAttribute("data-question"));
    expect(order).toEqual(["q-waiting", "q-remark", "q-default", "q-rest"]);
    const heading = within(details).getByRole("heading", { name: /^Questions/ }).textContent!.replace(/\s+/g, " ");
    expect(heading).toBe("Questions 4 · 1 open");
    expect(within(questionEl(details, "q-default")).getByText("By default")).toBeTruthy();
    expect(within(questionEl(details, "q-rest")).queryByText("By default")).toBeNull();
  });

  test("shows the current answer once with who and when, and the earlier one struck through beneath it with who and when", async () => {
    vi.useFakeTimers({ toFake: ["Date"], now: new Date(TODAY) });
    fakeNode({
      questions: [
        question("q", "How many results show?", {
          answers: [answer("Five", AGENT, "agent", LONG_AGO), answer("Twenty", PERSON_ME, "human", TODAY)],
        }),
      ],
    });
    const details = renderChosen();
    await waitFor(() => expect(details.querySelector("[data-question]")).not.toBeNull());

    const asked = questionEl(details, "q");
    expect([...asked.querySelectorAll("s")].map((one) => one.textContent)).toEqual(["Five"]);
    expect(within(asked).getAllByText("Twenty")).toHaveLength(1);
    const current = within(asked).getByText("Twenty").closest("div")!.nextElementSibling!;
    expect(current.textContent).toContain("You");
    expect(current.textContent).toContain("today");
    const earlier = asked.querySelector("s")!.parentElement!;
    expect(earlier.textContent).toContain("Agent r7/grill, nobody was asked");
    expect(earlier.textContent).toContain("4 Mar 2025");
    expect(earlier.textContent).not.toContain("today");
  });

  test("says when the node could not be loaded, under the title from the list", async () => {
    fakeNode({ status: 500 });
    const details = renderChosen();

    expect(await within(details).findByText("The node could not be loaded. Try again in a moment.")).toBeTruthy();
    expect(details.querySelector("h2")!.textContent).toBe("No player is found");
  });

  test("shows the path as links that choose each node above, then the full title", async () => {
    fakeNode({});
    const onChoose = vi.fn();
    const details = renderChosen(TREE[3], TREE, onChoose);

    const path = within(details).getAllByRole("button").slice(0, 3).map((one) => one.textContent);
    expect(path).toEqual(["Disc golf putting trainer", "Player: add a friend", "Player searches for a player"]);
    fireEvent.click(within(details).getByRole("button", { name: "Player: add a friend" }));
    expect(onChoose).toHaveBeenCalledWith("story");
    expect(details.querySelector("h2")!.textContent).toBe("No player is found");
  });

  test("shows a status pill, the story pill and the count answered by default only when it is above 0", async () => {
    fakeNode({});
    const pills = (details: HTMLElement) => [...details.querySelector("h2")!.nextElementSibling!.children].map((one) => one.textContent);
    const story = listed("story", "feature", 1, TREE[1].title, { status: "open", default_answers: 2 });
    let details = renderChosen(story, [TREE[0], story, TREE[2], TREE[3]]);
    expect(pills(details)).toEqual(["Open", "User story", "2 answered by default"]);
    cleanup();

    details = renderChosen(listed("none", "search", 3, "No player is found", { status: "out_of_scope" }));
    expect(pills(details)).toEqual(["Out of scope"]);
  });

  test("says which other nodes point at the node, as links that choose them", async () => {
    fakeNode({});
    const target = listed("none", "search", 3, "No player is found");
    const tree = [
      ...TREE.slice(0, 3),
      target,
      listed("same", "search", 3, "Player searches with an empty name", { same_as: { id: "none" } as TreeNode["same_as"] }),
      listed("handoff", "search", 3, "Player cancels the search", { handoff: { id: "none" } as TreeNode["handoff"] }),
      listed("elsewhere", "search", 3, "Player finds a friend", { same_as: { id: "search" } as TreeNode["same_as"] }),
    ];
    const onChoose = vi.fn();
    const details = renderChosen(target, tree, onChoose);

    const reached = details.querySelector("details")!;
    expect(reached.querySelector("summary")!.textContent).toBe("Reached from 2 other places");
    expect(within(reached).getAllByRole("button").map((one) => one.textContent)).toEqual([
      "Player searches with an empty name",
      "Player cancels the search",
    ]);
    fireEvent.click(within(reached).getByRole("button", { name: "Player cancels the search" }));
    expect(onChoose).toHaveBeenCalledWith("handoff");
  });

  test("a node with a pointer shows Same as or Hands off to with a link that chooses the target", async () => {
    fakeNode({});
    const pointing = listed("none", "search", 3, "No player is found", { same_as: { id: "search" } as TreeNode["same_as"] });
    const onChoose = vi.fn();
    const details = renderChosen(pointing, [...TREE.slice(0, 3), pointing], onChoose);

    const line = within(details).getByText(/↪ Same as/);
    fireEvent.click(within(line).getByRole("button", { name: "Player searches for a player" }));
    expect(onChoose).toHaveBeenCalledWith("search");
    expect(within(details).queryByText(/Hands off to/)).toBeNull();
  });
});

describe("a node's text", () => {
  test("keeps lists, bold and code, shows a link by its label only and HTML as plain text", () => {
    const { container } = render(<Text source={"See [the rules](https://example.com) and <b>this</b>\n- **Bold** item\n- `code` item"} />);
    expect(container.querySelector("a")).toBeNull();
    expect(container.querySelector("b")).toBeNull();
    expect(container.querySelector("p")!.textContent).toBe("See the rules and <b>this</b>");
    expect([...container.querySelectorAll("li")].map((one) => one.textContent)).toEqual(["Bold item", "code item"]);
    expect(container.querySelector("li strong")!.textContent).toBe("Bold");
    expect(container.querySelector("li code")!.textContent).toBe("code");
  });
});

describe("the details with nothing chosen", () => {
  test("say that no question waits, 1 question waits, or N questions wait, with a link to show only the open nodes", () => {
    const props = { node: null, tree: TREE, me: ME, onChoose: () => {}, workspaceId: WORKSPACE_ID, projectId: PROJECT_ID, featureId: FEATURE_ID };
    const { rerender } = render(<Details {...props} waitingCount={0} />);
    const details = screen.getByRole("region", { name: "Details" });
    expect(details.textContent).toContain("No question waits for an answer.");
    expect(within(details).getByRole("button", { name: "Show only the open nodes" })).toBeTruthy();
    rerender(<Details {...props} waitingCount={1} />);
    expect(details.textContent).toContain("1 question waits for an answer.");
    rerender(<Details {...props} waitingCount={3} />);
    expect(details.textContent).toContain("3 questions wait for an answer.");
  });
});
