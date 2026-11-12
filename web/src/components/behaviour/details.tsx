import { useCallback, useState } from "react";
import { answerQuestion, loadNode, type TreeNode } from "@/api/behaviour";
import { ApiError } from "@/api/client";
import type { AnswerBody, QuestionBody } from "@/api/generated";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useReloading } from "@/lib/reloading";
import { cn } from "@/lib/utils";
import { Text } from "./text";
import { ancestorsOf, descendantsOf, shortTitle } from "./tree";

// The chosen node, read in full: path, title, status, text, questions with their answers, rules.

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

/** "today", otherwise day, short month and year. */
function fmtDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  if (date.toDateString() === new Date().toDateString()) return "today";
  return date.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
}

const agentName = (author: Record<string, unknown>) => `${String(author.run ?? "")}/${String(author.role ?? "")}`;

/** Who gave an answer, without names or emails: "You", "A member", or the agent by run and role. */
export function who(answer: Pick<AnswerBody, "author" | "given_by">, me: string | null): string {
  const { author } = answer;
  if (author.kind === "human") return author.user_id === me ? "You" : "A member";
  if (answer.given_by === "human") return `A person, written down by agent ${agentName(author)}`;
  return `Agent ${agentName(author)}, nobody was asked`;
}

const CONFLICT = "Someone answered this first; it has been reloaded.";
const CHANGE_CONFLICT = "Someone changed this first; it has been reloaded.";

const pill = "h-5 whitespace-nowrap rounded-full bg-muted px-2 text-[11.5px] font-medium leading-5 text-muted-foreground";

// Waiting questions, then a person's remarks, then answers by default, then the rest.
function rank(question: QuestionBody): number {
  if (question.state === "open") return question.asked_by.kind === "human" ? 1 : 0;
  return question.answer?.given_by === "agent" ? 2 : 3;
}

function Link({ children, onClick }: { children: string; onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} className="cursor-pointer underline decoration-input underline-offset-[3px] hover:decoration-foreground">
      {children}
    </button>
  );
}

/** The details of `node`; with none chosen, how to choose one and how many questions wait. */
export function Details({
  node,
  tree,
  me,
  onChoose,
  workspaceId,
  projectId,
  featureId,
  waitingCount = 0,
  onShowOpen,
  onWritten,
}: {
  node: TreeNode | null;
  tree: TreeNode[];
  me: string | null;
  onChoose: (id: string) => void;
  workspaceId: string;
  projectId: string;
  featureId: string;
  waitingCount?: number;
  onShowOpen?: () => void;
  /** After a write, accepted or not: the tree and the open questions need reading again. */
  onWritten?: () => void;
}) {
  return (
    <section aria-label="Details" className="min-h-0 overflow-y-auto border-l">
      {node ? (
        <Chosen
          key={node.id}
          node={node}
          tree={tree}
          me={me}
          onChoose={onChoose}
          ids={{ workspaceId, projectId, featureId }}
          onWritten={onWritten ?? (() => {})}
        />
      ) : (
        <p className="px-6 py-10 text-[13px] leading-relaxed text-muted-foreground">
          Choose a node in the folders or in the graph to read it here: its text, its questions with their answers, and its rules.
          <br />
          <br />
          {waitingCount === 0 ? "No question waits for an answer." : `${plural(waitingCount, "question waits", "questions wait")} for an answer.`}{" "}
          <Link onClick={onShowOpen ?? (() => {})}>Show only the open nodes</Link>
        </p>
      )}
    </section>
  );
}

function Chosen({
  node: listed,
  tree,
  me,
  onChoose,
  ids,
  onWritten,
}: {
  node: TreeNode;
  tree: TreeNode[];
  me: string | null;
  onChoose: (id: string) => void;
  ids: { workspaceId: string; projectId: string; featureId: string };
  onWritten: () => void;
}) {
  const load = useCallback(
    () => loadNode(ids.workspaceId, ids.projectId, ids.featureId, listed.id),
    [ids.workspaceId, ids.projectId, ids.featureId, listed.id],
  );
  const { value: read, failed, reload } = useReloading(load);
  // What to say under a question after a write; it lasts until another node is chosen.
  const [said, setSaid] = useState<Record<string, string>>({});
  const written = (id: string, message: string) => {
    setSaid((before) => ({ ...before, [id]: message }));
    reload();
    onWritten();
  };
  const byId = new Map(tree.map((one) => [one.id, one]));
  const label = (id: string) => (byId.has(id) ? shortTitle(byId.get(id)!) : "");
  const crumbs = ancestorsOf(tree, listed.id).reverse();
  const reachedFrom = tree.filter((one) => one.same_as?.id === listed.id || one.handoff?.id === listed.id);
  const node = read?.node;
  const level = node?.level ?? listed.level;
  const status = listed.status;
  const defaults = listed.default_answers;

  const pointer = (glyph: string, words: string, target: { id: string; title?: string } | null | undefined) =>
    target && (
      <p className="mt-2.5 text-[12.5px] text-muted-foreground">
        {glyph} {words}{" "}
        {byId.has(target.id) ? <Link onClick={() => onChoose(target.id)}>{label(target.id)}</Link> : <span>{target.title}</span>}
      </p>
    );

  return (
    <div className="px-5 pb-24 pt-4">
      <p className="text-[11.5px] leading-normal text-muted-foreground">
        {crumbs.length === 0 && "Feature"}
        {crumbs.map((id, at) => (
          <span key={id}>
            {at > 0 && " › "}
            <button type="button" onClick={() => onChoose(id)} className="cursor-pointer hover:text-foreground hover:underline">
              {label(id)}
            </button>
          </span>
        ))}
      </p>
      <h2 className="mt-1 text-[15px] font-semibold leading-snug">{listed.title}</h2>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {listed.needs_pass ? (
          <span className={cn(pill, "bg-transparent text-foreground shadow-[inset_0_0_0_1px_var(--foreground)]")}>↻ Needs another grill pass</span>
        ) : (
          <span className={cn(pill, status === "open" && "bg-signal-soft text-foreground")}>
            {status === "open" ? "Open" : status === "out_of_scope" ? "Out of scope" : "Done"}
          </span>
        )}
        {level === 1 && <span className={pill}>User story</span>}
        {level === 0 && <span className={pill}>Feature</span>}
        {defaults > 0 && <span className={pill}>{defaults} answered by default</span>}
      </div>
      {listed.needs_pass && (
        <p className="mt-3 rounded-lg border border-dashed border-foreground px-3 py-2.5 text-[12.5px] leading-normal">
          Waits for the next grill pass{listed.needs_pass_since ? ` since ${fmtDate(listed.needs_pass_since)}` : ""}. Something here or above was
          answered, changed or remarked. Nothing is rewritten until the grill is started again on the feature.
        </p>
      )}
      {listed.summary && <p className="mt-3 text-[13.5px] leading-relaxed">{listed.summary}</p>}
      {pointer("↪", "Same as", node?.same_as ?? (listed.same_as && { id: listed.same_as.id }))}
      {pointer("⇢", "Hands off to", node?.handoff ?? (listed.handoff && { id: listed.handoff.id }))}
      {reachedFrom.length > 0 && (
        <details className="mt-1.5 text-xs text-muted-foreground">
          <summary className="cursor-pointer py-1">Reached from {plural(reachedFrom.length, "other place", "other places")}</summary>
          <div className="space-y-1">
            {reachedFrom.map((one) => (
              <div key={one.id}>
                <Link onClick={() => onChoose(one.id)}>{shortTitle(one)}</Link>
              </div>
            ))}
          </div>
        </details>
      )}
      {failed && (
        <Alert variant="destructive" className="mt-4">
          <AlertDescription>The node could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {read && (
        <>
          {read.node.situation && (
            <>
              <h3 className="mt-5 text-[11.5px] font-semibold uppercase tracking-wide text-muted-foreground">Situation</h3>
              <Text source={read.node.situation} className="mt-1.5 space-y-2 text-[13px] leading-relaxed" />
            </>
          )}
          {read.node.behaviour && (
            <>
              <h3 className="mt-5 text-[11.5px] font-semibold uppercase tracking-wide text-muted-foreground">Behaviour</h3>
              <Text source={read.node.behaviour} className="mt-1.5 space-y-2 text-[13px] leading-relaxed" />
            </>
          )}
          <Questions questions={read.questions} me={me} ids={ids} said={said} below={descendantsOf(tree, listed.id).length} onWritten={written} />
          <Rules rules={read.rules} nodeId={read.node.id} byId={byId} />
        </>
      )}
    </div>
  );
}

type Ids = { workspaceId: string; projectId: string; featureId: string };

/** The field and button under a waiting question. */
function AnswerField({
  question,
  ids,
  onWritten,
}: {
  question: QuestionBody;
  ids: Ids;
  onWritten: (id: string, message: string) => void;
}) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const send = async () => {
    const answer = text.trim();
    if (!answer || busy) return;
    setBusy(true);
    setError("");
    try {
      await answerQuestion(ids.workspaceId, ids.projectId, ids.featureId, question.id, { expected_version: question.version, answer });
      onWritten(question.id, "Recorded. This node now waits for the next grill pass.");
    } catch (failure) {
      setBusy(false);
      if (failure instanceof ApiError && failure.status === 409) {
        onWritten(question.id, CONFLICT);
      } else setError("The answer could not be saved. Try again in a moment.");
    }
  };
  return (
    <>
      <textarea
        aria-label="Your answer"
        placeholder="Your answer"
        value={text}
        onChange={(event) => setText(event.target.value)}
        className="mt-2 block min-h-16 w-full resize-y rounded-lg border border-input bg-background px-2.5 py-2 text-[13.5px] leading-normal"
      />
      <div className="mt-2 flex items-center gap-2">
        <Button type="button" size="sm" disabled={busy || text.trim() === ""} onClick={send}>
          Answer
        </Button>
        <span className="text-xs text-muted-foreground">Nobody has answered this yet.</span>
      </div>
      {error && <p className="mt-2 text-xs text-foreground">{error}</p>}
    </>
  );
}

/** The Change button under an answered question, and the field it opens. */
function ChangeField({
  question,
  ids,
  below,
  onWritten,
}: {
  question: QuestionBody;
  ids: Ids;
  below: number;
  onWritten: (id: string, message: string) => void;
}) {
  const current = question.answer?.answer ?? "";
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const start = () => {
    setText(current);
    setError("");
    setOpen(true);
  };
  const save = async () => {
    const answer = text.trim();
    if (busy) return;
    if (!answer || answer === current.trim()) {
      setOpen(false);
      return;
    }
    setBusy(true);
    setError("");
    try {
      await answerQuestion(ids.workspaceId, ids.projectId, ids.featureId, question.id, { expected_version: question.version, answer, change: true });
      setBusy(false);
      setOpen(false);
      onWritten(
        question.id,
        below > 0 ? `Recorded. This node and the ${below} below it now wait for the next grill pass.` : "Recorded. This node now waits for the next grill pass.",
      );
    } catch (failure) {
      setBusy(false);
      if (failure instanceof ApiError && failure.status === 409) {
        setOpen(false);
        onWritten(question.id, CHANGE_CONFLICT);
      } else setError("The answer could not be saved. Try again in a moment.");
    }
  };
  if (!open) {
    return (
      <Button type="button" variant="ghost" size="sm" className="mt-1.5 -ml-2 h-6 px-2 text-xs" onClick={start}>
        Change
      </Button>
    );
  }
  return (
    <>
      <textarea
        aria-label="New answer"
        value={text}
        onChange={(event) => setText(event.target.value)}
        className="mt-2 block min-h-16 w-full resize-y rounded-lg border border-input bg-background px-2.5 py-2 text-[13.5px] leading-normal"
      />
      <div className="mt-2 flex items-center gap-2">
        <Button type="button" size="sm" disabled={busy} onClick={save}>
          Save new answer
        </Button>
        <Button type="button" size="sm" variant="ghost" disabled={busy} onClick={() => setOpen(false)}>
          Cancel
        </Button>
      </div>
      <p className="mt-2 text-xs leading-normal text-muted-foreground">
        The old answer is kept under the new one.
        {below > 0 && ` Saving marks this node and the ${below} below it for the next grill pass.`}
      </p>
      {error && <p className="mt-2 text-xs text-foreground">{error}</p>}
    </>
  );
}

function Questions({
  questions,
  me,
  ids,
  said,
  below,
  onWritten,
}: {
  questions: QuestionBody[];
  me: string | null;
  ids: Ids;
  said: Record<string, string>;
  below: number;
  onWritten: (id: string, message: string) => void;
}) {
  const sorted = questions.map((question, at) => ({ question, at })).sort((a, b) => rank(a.question) - rank(b.question) || a.at - b.at);
  const waiting = questions.filter((question) => question.state === "open" && question.asked_by.kind !== "human").length;
  return (
    <>
      <h3 className="mt-5 text-[11.5px] font-semibold uppercase tracking-wide text-muted-foreground">
        Questions <span className="tabular-nums">{questions.length}</span>
        {waiting > 0 && <span> · {waiting} open</span>}
      </h3>
      {questions.length === 0 && <p className="mt-1 text-xs text-muted-foreground">No question has been asked on this node.</p>}
      {sorted.map(({ question }) => {
        const answer = question.answer;
        const earlier = question.answers.slice(0, -1).reverse();
        const waits = question.state === "open" && question.asked_by.kind !== "human";
        return (
          <div key={question.id} data-question={question.id} className={cn("mt-2 border-t py-3", waits && "-mx-2 rounded-md border-t-0 bg-signal-soft px-2")}>
            <p className="text-[13.5px] font-medium leading-snug">{question.question}</p>
            {question.state === "open" ? (
              waits ? (
                <AnswerField question={question} ids={ids} onWritten={onWritten} />
              ) : (
              <p className="mt-1.5 text-xs text-muted-foreground">
                {`${who({ author: question.asked_by, given_by: "human" }, me)} · ${fmtDate(question.created_at)} · Remark, waits for the next grill pass`}
              </p>
              )
            ) : (
              answer && (
                <>
                  <Text source={answer.answer} className="mt-1 space-y-1 text-[13.5px] leading-relaxed" />
                  <p className="mt-1.5 flex flex-wrap items-center gap-2 text-[11.5px] text-muted-foreground">
                    {answer.given_by === "agent" && <span className={cn(pill, "bg-transparent shadow-[inset_0_0_0_1px_var(--input)]")}>By default</span>}
                    <span>
                      {who(answer, me)} · {fmtDate(answer.at)}
                    </span>
                  </p>
                  <ChangeField question={question} ids={ids} below={below} onWritten={onWritten} />
                </>
              )
            )}
            {said[question.id] && (
              <p className="mt-2 flex gap-1.5 text-xs leading-normal">
                {said[question.id] !== CONFLICT && said[question.id] !== CHANGE_CONFLICT && <span className="font-semibold">↻</span>}
                {said[question.id]}
              </p>
            )}
            {earlier.map((one, at) => (
              <div key={at} className="mt-2 border-l-2 pl-2.5 text-[12.5px] leading-normal text-muted-foreground">
                <s className="decoration-input">{one.answer}</s>
                <br />
                Earlier answer · {who(one, me)} · {fmtDate(one.at)}
              </div>
            ))}
          </div>
        );
      })}
    </>
  );
}

const reach: Record<string, string> = { node: "This node", story: "This story", feature: "Whole feature" };

function Rules({ rules, nodeId, byId }: { rules: { id: string; node_id: string; scope: string; text: string }[]; nodeId: string; byId: Map<string, TreeNode> }) {
  const own = rules.filter((rule) => rule.node_id === nodeId);
  const above = rules.filter((rule) => rule.node_id !== nodeId);
  const written = (id: string) => {
    const holder = byId.get(id);
    if (!holder) return "";
    return holder.level === 0 ? "Feature" : holder.level === 1 ? "Story" : shortTitle(holder);
  };
  return (
    <>
      <h3 className="mt-5 text-[11.5px] font-semibold uppercase tracking-wide text-muted-foreground">
        Rules <span className="tabular-nums">{own.length}</span>
      </h3>
      {own.length === 0 && <p className="mt-1 text-xs text-muted-foreground">This node sets no rule of its own.</p>}
      {own.map((rule) => (
        <div key={rule.id} className="flex gap-2.5 border-t py-1.5 text-[13px] leading-normal first:border-t-0">
          <span className="w-[78px] flex-none pt-0.5 text-[11.5px] text-muted-foreground">{reach[rule.scope] ?? rule.scope}</span>
          <span>{rule.text}</span>
        </div>
      ))}
      {above.length > 0 && (
        <details className="mt-1.5 text-xs text-muted-foreground">
          <summary className="cursor-pointer py-1">{above.length === 1 ? "1 rule from above also holds here" : `${above.length} rules from above also hold here`}</summary>
          {above.map((rule) => (
            <div key={rule.id} className="flex gap-2.5 border-t py-1.5 text-[13px] leading-normal text-foreground">
              <span className="w-[78px] flex-none pt-0.5 text-[11.5px] text-muted-foreground">{written(rule.node_id)}</span>
              <span>{rule.text}</span>
            </div>
          ))}
        </details>
      )}
    </>
  );
}
