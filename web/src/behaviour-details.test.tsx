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
    expect(within(questionEl(details, "q-remark")).queryByRole("textbox", { name: "Your answer" })).toBeNull();
    expect(within(questionEl(details, "q-waiting")).getByRole("textbox", { name: "Your answer" })).toBeTruthy();
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
    cleanup();

    details = renderChosen(TREE[0]);
    expect(pills(details)).toEqual(["Done", "Feature"]);
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

  test("a node with a pointer shows ↪ Same as or ⇢ Hands off to with a link that chooses the target, and not the other", async () => {
    fakeNode({});
    const onChoose = vi.fn();
    const same = listed("none", "search", 3, "No player is found", { same_as: { id: "search" } as TreeNode["same_as"] });
    let details = renderChosen(same, [...TREE.slice(0, 3), same], onChoose);

    let line = within(details).getByText(/↪ Same as/);
    fireEvent.click(within(line).getByRole("button", { name: "Player searches for a player" }));
    expect(onChoose).toHaveBeenCalledWith("search");
    expect(within(details).queryByText(/Hands off to/)).toBeNull();
    cleanup();

    const handoff = listed("none", "search", 3, "No player is found", { handoff: { id: "story" } as TreeNode["handoff"] });
    details = renderChosen(handoff, [...TREE.slice(0, 3), handoff], onChoose);

    line = within(details).getByText(/⇢ Hands off to/);
    fireEvent.click(within(line).getByRole("button", { name: "Player: add a friend" }));
    expect(onChoose).toHaveBeenLastCalledWith("story");
    expect(within(details).queryByText(/Same as/)).toBeNull();
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

// US-228: a person answers a waiting question in the details with
// POST .../behaviour/questions/<question>/answers, then the node is read again.
describe("answering a waiting question", () => {
  const ANSWERS_PATH = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/features/${FEATURE_ID}/behaviour/questions/q-wait/answers`;
  const WAITING = { ...question("q-wait", "What does a search result show?", { state: "open" }), version: 3 };
  const ANSWERED = question("q-wait", "What does a search result show?", { answers: [answer("Name and handicap", PERSON_ME, "human", TODAY)] });

  /**
   * GET .../behaviour/nodes/search answers with `questions()` as they are at
   * that moment; a POST of an answer is handed to `post` with its body.
   */
  function fakeSearch(questions: () => unknown[], post: (body: unknown) => Promise<Response> | Response) {
    const read = () => ({
      node: { id: "search", parent_id: "story", path: "search", position: 0, level: 2, title: "Player searches for a player", summary: "", status: "open", situation: "", behaviour: "", needs_pass: false, created_at: LONG_AGO, created_by: AGENT, updated_at: LONG_AGO, version: 1 },
      chain: [],
      children: [],
      siblings: [],
      questions: questions(),
      rules: [],
    });
    const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        if (url === `${NODE_PATH}search` && (init?.method ?? "GET") === "GET") return json(200, read());
        if (url === ANSWERS_PATH && init?.method === "POST") return post(JSON.parse(String(init.body)));
        return json(404, { error: "not_found" });
      }),
    );
  }

  const accepted = (body: unknown) => new Response(JSON.stringify({ question: body }), { status: 201, headers: { "content-type": "application/json" } });

  async function fieldOf(details: HTMLElement) {
    await waitFor(() => expect(within(questionEl(details, "q-wait")).queryByRole("textbox", { name: "Your answer" })).not.toBeNull());
    const waiting = questionEl(details, "q-wait");
    return { field: within(waiting).getByRole("textbox", { name: "Your answer" }) as HTMLTextAreaElement, button: within(waiting).getByRole("button", { name: "Answer" }) as HTMLButtonElement };
  }

  test("on a node with nodes below it says only this node now waits for the next grill pass", async () => {
    let questions: unknown[] = [WAITING];
    fakeSearch(
      () => questions,
      () => {
        questions = [ANSWERED];
        return accepted(ANSWERED);
      },
    );
    const details = renderChosen(TREE[2]);
    const { field, button } = await fieldOf(details);

    fireEvent.change(field, { target: { value: "Name and handicap" } });
    fireEvent.click(button);

    await waitFor(() => expect(questionEl(details, "q-wait").textContent).toContain("Recorded. This node now waits for the next grill pass."));
    expect(details.textContent).not.toContain("below it");
  });

  test("sends the trimmed text with the question's version, and Answer is disabled while the text is only spaces", async () => {
    const sent: unknown[] = [];
    fakeSearch(
      () => [WAITING],
      (body) => {
        sent.push(body);
        return accepted(ANSWERED);
      },
    );
    const details = renderChosen(TREE[2]);
    const { field, button } = await fieldOf(details);
    expect(button.disabled).toBe(true);

    fireEvent.change(field, { target: { value: "   " } });
    expect(button.disabled).toBe(true);

    fireEvent.change(field, { target: { value: "  Name and handicap \n" } });
    expect(button.disabled).toBe(false);
    fireEvent.click(button);

    await waitFor(() => expect(sent).toEqual([{ expected_version: 3, answer: "Name and handicap" }]));
  });

  test("keeps Answer disabled while the answer is on its way, and on a failure other than 409 says it could not be saved and keeps the typed text", async () => {
    let fail: (response: Response) => void = () => {};
    fakeSearch(
      () => [WAITING],
      () => new Promise<Response>((resolve) => (fail = resolve)),
    );
    const details = renderChosen(TREE[2]);
    const { field, button } = await fieldOf(details);

    fireEvent.change(field, { target: { value: "Name and handicap" } });
    fireEvent.click(button);
    expect(button.disabled).toBe(true);

    fail(new Response(JSON.stringify({ error: "internal" }), { status: 500, headers: { "content-type": "application/json" } }));

    await waitFor(() => expect(questionEl(details, "q-wait").textContent).toContain("The answer could not be saved. Try again in a moment."));
    const after = within(questionEl(details, "q-wait")).getByRole("textbox", { name: "Your answer" }) as HTMLTextAreaElement;
    expect(after.value).toBe("Name and handicap");
    expect(within(questionEl(details, "q-wait")).getByRole("button", { name: "Answer" })).toHaveProperty("disabled", false);
  });

  test("the line under the answered question goes once another node is chosen and this one is chosen again", async () => {
    let questions: unknown[] = [WAITING];
    fakeSearch(
      () => questions,
      () => {
        questions = [ANSWERED];
        return accepted(ANSWERED);
      },
    );
    const props = { tree: TREE, me: ME, onChoose: () => {}, workspaceId: WORKSPACE_ID, projectId: PROJECT_ID, featureId: FEATURE_ID };
    const { rerender } = render(<Details node={TREE[2]} {...props} />);
    const details = screen.getByRole("region", { name: "Details" });
    const { field, button } = await fieldOf(details);
    fireEvent.change(field, { target: { value: "Name and handicap" } });
    fireEvent.click(button);
    await waitFor(() => expect(details.textContent).toContain("Recorded."));

    rerender(<Details node={TREE[3]} {...props} />);
    rerender(<Details node={TREE[2]} {...props} />);

    await waitFor(() => expect(questionEl(details, "q-wait")).not.toBeNull());
    expect(details.textContent).not.toContain("Recorded.");
  });
});

// US-229: any answered question, also one given by default, can be changed with
// POST .../behaviour/questions/<question>/answers and `change: true`. A change
// marks the node and every node below it for the next grill pass.
describe("changing an answer", () => {
  const ANSWERS_PATH = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/features/${FEATURE_ID}/behaviour/questions/q-default/answers`;
  const BY_DEFAULT = { ...question("q-default", "What does a search result show?", { answers: [answer("Name only", AGENT, "agent")] }), version: 4 };
  const CHANGED_BY_OTHER = question("q-default", "What does a search result show?", {
    answers: [answer("Name only", AGENT, "agent"), answer("Name and club", PERSON_OTHER, "human", TODAY)],
  });
  // `search` has one child and one grandchild below it: 2 nodes below.
  const DEEP_TREE = [...TREE, listed("deeper", "none", 4, "The search is cleared")];
  // Without `none`, `search` has nothing below it.
  const LEAF_TREE = TREE.slice(0, 3);

  /**
   * GET .../behaviour/nodes/search answers with `questions()` as they are at
   * that moment; a POST to the question's answers is handed to `post` with its body.
   */
  function fakeSearch(questions: () => unknown[], post: (body: unknown) => Promise<Response> | Response) {
    const read = () => ({
      node: { id: "search", parent_id: "story", path: "search", position: 0, level: 2, title: "Player searches for a player", summary: "", status: "answered", situation: "", behaviour: "", needs_pass: false, created_at: LONG_AGO, created_by: AGENT, updated_at: LONG_AGO, version: 1 },
      chain: [],
      children: [],
      siblings: [],
      questions: questions(),
      rules: [],
    });
    const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        if (url === `${NODE_PATH}search` && (init?.method ?? "GET") === "GET") return json(200, read());
        if (url === ANSWERS_PATH && init?.method === "POST") return post(JSON.parse(String(init.body)));
        return json(404, { error: "not_found" });
      }),
    );
  }

  const accepted = (body: unknown) => new Response(JSON.stringify({ question: body }), { status: 201, headers: { "content-type": "application/json" } });

  async function openChange(details: HTMLElement) {
    await waitFor(() => expect(questionEl(details, "q-default")).not.toBeNull());
    fireEvent.click(within(questionEl(details, "q-default")).getByRole("button", { name: "Change" }));
    return within(questionEl(details, "q-default")).getByRole("textbox", { name: "New answer" }) as HTMLTextAreaElement;
  }

  const save = (details: HTMLElement) => fireEvent.click(within(questionEl(details, "q-default")).getByRole("button", { name: "Save new answer" }));

  test("an answer given by default opens in the field, and saving sends the trimmed text as a change and says this node and every node below it wait", async () => {
    const sent: unknown[] = [];
    fakeSearch(
      () => [BY_DEFAULT],
      (body) => {
        sent.push(body);
        return accepted(BY_DEFAULT);
      },
    );
    const details = renderChosen(TREE[2], DEEP_TREE);
    const field = await openChange(details);
    expect(field.value).toBe("Name only");
    expect(questionEl(details, "q-default").textContent).toContain("The old answer is kept under the new one. Saving marks this node and the 2 below it for the next grill pass.");

    fireEvent.change(field, { target: { value: "  Name and handicap \n" } });
    save(details);

    await waitFor(() => expect(questionEl(details, "q-default").textContent).toContain("Recorded. This node and the 2 below it now wait for the next grill pass."));
    expect(sent).toEqual([{ expected_version: 4, answer: "Name and handicap", change: true }]);
    expect(within(questionEl(details, "q-default")).queryByRole("textbox", { name: "New answer" })).toBeNull();
  });

  test("on a node with nothing below it leaves out the nodes below, before and after saving", async () => {
    fakeSearch(
      () => [BY_DEFAULT],
      () => accepted(BY_DEFAULT),
    );
    const details = renderChosen(TREE[2], LEAF_TREE);
    const field = await openChange(details);
    expect(questionEl(details, "q-default").textContent).toContain("The old answer is kept under the new one.");
    expect(questionEl(details, "q-default").textContent).not.toContain("Saving marks");

    fireEvent.change(field, { target: { value: "Name and handicap" } });
    save(details);

    await waitFor(() => expect(questionEl(details, "q-default").textContent).toContain("Recorded. This node now waits for the next grill pass."));
    expect(details.textContent).not.toContain("below it");
  });

  test("saving the same text with spaces around it, saving an empty text and Cancel each close the field and send nothing", async () => {
    const sent: unknown[] = [];
    fakeSearch(
      () => [BY_DEFAULT],
      (body) => {
        sent.push(body);
        return accepted(BY_DEFAULT);
      },
    );
    const details = renderChosen(TREE[2], DEEP_TREE);
    const closed = () => within(questionEl(details, "q-default")).queryByRole("textbox", { name: "New answer" });

    fireEvent.change(await openChange(details), { target: { value: "  Name only \n" } });
    save(details);
    expect(closed()).toBeNull();

    fireEvent.change(await openChange(details), { target: { value: "   " } });
    save(details);
    expect(closed()).toBeNull();

    fireEvent.change(await openChange(details), { target: { value: "Name and handicap" } });
    fireEvent.click(within(questionEl(details, "q-default")).getByRole("button", { name: "Cancel" }));
    expect(closed()).toBeNull();

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(sent).toEqual([]);
    expect(details.textContent).not.toContain("Recorded.");
  });

  test("a 409 closes the field, reads the node again and shows the other member's answer with the earlier one beneath", async () => {
    let questions: unknown[] = [BY_DEFAULT];
    fakeSearch(
      () => questions,
      () => {
        questions = [CHANGED_BY_OTHER];
        return new Response(JSON.stringify({ error: "version_conflict" }), { status: 409, headers: { "content-type": "application/json" } });
      },
    );
    const details = renderChosen(TREE[2], DEEP_TREE);
    fireEvent.change(await openChange(details), { target: { value: "Name and handicap" } });
    save(details);

    await waitFor(() => expect(questionEl(details, "q-default").textContent).toContain("Name and club"));
    const changed = questionEl(details, "q-default");
    expect(changed.textContent).toContain("Someone changed this first; it has been reloaded.");
    expect(changed.textContent).not.toContain("Recorded.");
    expect(within(changed).queryByRole("textbox", { name: "New answer" })).toBeNull();
    expect(changed.querySelector("s")!.textContent).toBe("Name only");
  });

  test("on a failure other than 409 says it could not be saved and keeps the field open with the typed text", async () => {
    fakeSearch(
      () => [BY_DEFAULT],
      () => new Response(JSON.stringify({ error: "internal" }), { status: 500, headers: { "content-type": "application/json" } }),
    );
    const details = renderChosen(TREE[2], DEEP_TREE);
    fireEvent.change(await openChange(details), { target: { value: "Name and handicap" } });
    save(details);

    await waitFor(() => expect(questionEl(details, "q-default").textContent).toContain("The answer could not be saved. Try again in a moment."));
    const field = within(questionEl(details, "q-default")).getByRole("textbox", { name: "New answer" }) as HTMLTextAreaElement;
    expect(field.value).toBe("Name and handicap");
    expect(within(questionEl(details, "q-default")).getByRole("button", { name: "Save new answer" })).toHaveProperty("disabled", false);
  });
});

// US-230: any node takes a person's question or remark with
// POST .../behaviour/nodes/<node>/questions. It opens the node again for the
// next grill pass; a person's open question is a remark and has no answer field.
describe("adding a question or remark", () => {
  const ASK_PATH = `${NODE_PATH}none/questions`;

  /**
   * GET .../behaviour/nodes/none answers with `questions()` as they are at that
   * moment; a POST of a question is handed to `post` with its body.
   */
  function fakeNone(questions: () => unknown[], post: (body: unknown) => Promise<Response> | Response) {
    const read = () => ({
      node: { id: "none", parent_id: "search", path: "none", position: 0, level: 3, title: "No player is found", summary: "", status: "answered", situation: "", behaviour: "", needs_pass: false, created_at: LONG_AGO, created_by: AGENT, updated_at: LONG_AGO, version: 1 },
      chain: [],
      children: [],
      siblings: [],
      questions: questions(),
      rules: [],
    });
    const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        if (url === `${NODE_PATH}none` && (init?.method ?? "GET") === "GET") return json(200, read());
        if (url === ASK_PATH && init?.method === "POST") return post(JSON.parse(String(init.body)));
        return json(404, { error: "not_found" });
      }),
    );
  }

  const REMARK = question("q-remark", "The confirmation is easy to miss.", { state: "open", asked_by: PERSON_ME });
  const accepted = () => new Response(JSON.stringify({ question: REMARK }), { status: 201, headers: { "content-type": "application/json" } });

  async function remarkField(details: HTMLElement) {
    const field = (await within(details).findByRole("textbox", { name: "Add a question or remark" })) as HTMLTextAreaElement;
    return { field, button: within(details).getByRole("button", { name: "Add to this node" }) as HTMLButtonElement };
  }

  test("sends the trimmed text to the node, and Add to this node is disabled while the text is empty or only spaces", async () => {
    const sent: unknown[] = [];
    fakeNone(
      () => [],
      (body) => {
        sent.push(body);
        return accepted();
      },
    );
    const details = renderChosen();
    const { field, button } = await remarkField(details);
    expect(button.disabled).toBe(true);

    fireEvent.change(field, { target: { value: "  \n " } });
    expect(button.disabled).toBe(true);

    fireEvent.change(field, { target: { value: "  The confirmation is easy to miss. \n" } });
    expect(button.disabled).toBe(false);
    fireEvent.click(button);

    await waitFor(() => expect(sent).toEqual([{ question: "The confirmation is easy to miss." }]));
  });

  test("keeps Add to this node disabled while the remark is on its way; a failure says it could not be saved and keeps the text, until pressed again", async () => {
    let answerWith: (response: Response) => void = () => {};
    fakeNone(
      () => [],
      () => new Promise<Response>((resolve) => (answerWith = resolve)),
    );
    const details = renderChosen();
    const { field, button } = await remarkField(details);

    fireEvent.change(field, { target: { value: "The confirmation is easy to miss." } });
    fireEvent.click(button);
    expect(button.disabled).toBe(true);

    answerWith(new Response(JSON.stringify({ error: "internal" }), { status: 500, headers: { "content-type": "application/json" } }));

    await waitFor(() => expect(details.textContent).toContain("The remark could not be saved. Try again in a moment."));
    expect(field.value).toBe("The confirmation is easy to miss.");
    expect(button.disabled).toBe(false);

    fireEvent.click(button);
    expect(details.textContent).not.toContain("The remark could not be saved.");
  });

  test("names a remark by the signed-in member You and one by another member A member", async () => {
    fakeNone(
      () => [
        question("q-mine", "The confirmation is easy to miss.", { state: "open", asked_by: PERSON_ME }),
        question("q-theirs", "The button is hard to reach.", { state: "open", asked_by: PERSON_OTHER }),
      ],
      accepted,
    );
    const details = renderChosen();
    await waitFor(() => expect(details.querySelectorAll("[data-question]")).toHaveLength(2));

    expect(questionEl(details, "q-mine").textContent).toContain("You ·");
    expect(questionEl(details, "q-mine").textContent).not.toContain("A member");
    expect(questionEl(details, "q-theirs").textContent).toContain("A member ·");
    expect(questionEl(details, "q-theirs").textContent).not.toContain("You");
  });
});

// US-235: while a grill runs the screen reads the chosen node again each time
// `refresh` changes. Text a person is typing stays; when an agent answers the
// question being typed to, the typed text stands in an open Change field.
describe("a live update of the chosen node", () => {
  const ANSWERS_PATH = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/features/${FEATURE_ID}/behaviour/questions/q-wait/answers`;
  const WAITING = { ...question("q-wait", "What does a search result show?", { state: "open" }), version: 3 };
  const ANSWERED_BY_DEFAULT = { ...question("q-wait", "What does a search result show?", { answers: [answer("Guest cart.", AGENT, "agent", TODAY)] }), version: 4 };
  const CHANGED_BY_OTHER = {
    ...question("q-wait", "What does a search result show?", { answers: [answer("Guest cart.", AGENT, "agent", TODAY), answer("Name and club", PERSON_OTHER, "human", TODAY)] }),
    version: 5,
  };
  const props = { tree: TREE, me: ME, onChoose: () => {}, workspaceId: WORKSPACE_ID, projectId: PROJECT_ID, featureId: FEATURE_ID };

  /**
   * GET .../behaviour/nodes/search answers with the questions in `server` as
   * they are at that moment; the body of each POST to q-wait's answers is kept
   * in `sent`.
   */
  function fakeSearch(server: { questions: unknown[] }) {
    const sent: unknown[] = [];
    const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        if (url === `${NODE_PATH}search` && (init?.method ?? "GET") === "GET")
          return json(200, {
            node: { id: "search", parent_id: "story", path: "search", position: 0, level: 2, title: "Player searches for a player", summary: "", status: "open", situation: "", behaviour: "", needs_pass: false, created_at: LONG_AGO, created_by: AGENT, updated_at: LONG_AGO, version: 1 },
            chain: [],
            children: [],
            siblings: [],
            questions: server.questions,
            rules: [],
          });
        if (url === ANSWERS_PATH && init?.method === "POST") {
          sent.push(JSON.parse(String(init.body)));
          return json(201, { question: server.questions[0] });
        }
        return json(404, { error: "not_found" });
      }),
    );
    return sent;
  }

  const q = (details: HTMLElement) => questionEl(details, "q-wait");
  const textbox = (details: HTMLElement, name: string) => within(q(details)).queryByRole("textbox", { name }) as HTMLTextAreaElement | null;

  test("an agent answering the question being typed to shows its answer By default with the typed text in an open Change field, and the remark text stays", async () => {
    const server = { questions: [WAITING] as unknown[] };
    const sent = fakeSearch(server);
    const { rerender } = render(<Details node={TREE[2]} {...props} refresh={0} />);
    const details = screen.getByRole("region", { name: "Details" });
    await waitFor(() => expect(textbox(details, "Your answer")).not.toBeNull());
    fireEvent.change(textbox(details, "Your answer")!, { target: { value: "Name and handicap" } });
    const remark = within(details).getByRole("textbox", { name: "Add a question or remark" }) as HTMLTextAreaElement;
    fireEvent.change(remark, { target: { value: "The results load slowly." } });

    server.questions = [ANSWERED_BY_DEFAULT];
    rerender(<Details node={TREE[2]} {...props} refresh={1} />);

    await waitFor(() => expect(q(details).textContent).toContain("Someone answered this first; it has been reloaded."));
    expect(textbox(details, "Your answer")).toBeNull();
    expect(within(q(details)).getByText("By default")).toBeTruthy();
    expect(q(details).textContent).toContain("Guest cart.");
    expect(textbox(details, "New answer")!.value).toBe("Name and handicap");
    expect(remark.value).toBe("The results load slowly.");

    fireEvent.click(within(q(details)).getByRole("button", { name: "Save new answer" }));
    await waitFor(() => expect(sent).toEqual([{ expected_version: 4, answer: "Name and handicap", change: true }]));
  });

  test("an agent answering a question whose field holds only spaces shows it answered, with no Change field open and no line", async () => {
    const server = { questions: [WAITING] as unknown[] };
    fakeSearch(server);
    const { rerender } = render(<Details node={TREE[2]} {...props} refresh={0} />);
    const details = screen.getByRole("region", { name: "Details" });
    await waitFor(() => expect(textbox(details, "Your answer")).not.toBeNull());
    fireEvent.change(textbox(details, "Your answer")!, { target: { value: "   " } });

    server.questions = [ANSWERED_BY_DEFAULT];
    rerender(<Details node={TREE[2]} {...props} refresh={1} />);

    await waitFor(() => expect(textbox(details, "Your answer")).toBeNull());
    // The line would come from an effect after the answered form is drawn.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(q(details).textContent).toContain("Guest cart.");
    expect(textbox(details, "New answer")).toBeNull();
    expect(details.textContent).not.toContain("Someone answered this first");
  });

  test("someone changing the answer being changed keeps the field open with the typed text, shows the new answer over the earlier one, and saves against the new version", async () => {
    const server = { questions: [ANSWERED_BY_DEFAULT] as unknown[] };
    const sent = fakeSearch(server);
    const { rerender } = render(<Details node={TREE[2]} {...props} refresh={0} />);
    const details = screen.getByRole("region", { name: "Details" });
    await waitFor(() => expect(q(details)).not.toBeNull());
    fireEvent.click(within(q(details)).getByRole("button", { name: "Change" }));
    fireEvent.change(textbox(details, "New answer")!, { target: { value: "Name and handicap" } });

    server.questions = [CHANGED_BY_OTHER];
    rerender(<Details node={TREE[2]} {...props} refresh={1} />);

    await waitFor(() => expect(q(details).textContent).toContain("Someone changed this first; it has been reloaded."));
    expect(q(details).textContent).toContain("Name and club");
    expect(q(details).querySelector("s")!.textContent).toBe("Guest cart.");
    expect(textbox(details, "New answer")!.value).toBe("Name and handicap");

    fireEvent.click(within(q(details)).getByRole("button", { name: "Save new answer" }));
    await waitFor(() => expect(sent).toEqual([{ expected_version: 5, answer: "Name and handicap", change: true }]));
  });
});
