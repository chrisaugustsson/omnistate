import { useCallback } from "react";
import { loadNode, type TreeNode } from "@/api/behaviour";
import type { AnswerBody, QuestionBody } from "@/api/generated";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useReloading } from "@/lib/reloading";
import { cn } from "@/lib/utils";
import { Text } from "./text";
import { ancestorsOf, shortTitle } from "./tree";

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
}: {
  node: TreeNode;
  tree: TreeNode[];
  me: string | null;
  onChoose: (id: string) => void;
  ids: { workspaceId: string; projectId: string; featureId: string };
}) {
  const load = useCallback(
    () => loadNode(ids.workspaceId, ids.projectId, ids.featureId, listed.id),
    [ids.workspaceId, ids.projectId, ids.featureId, listed.id],
  );
  const { value: read, failed } = useReloading(load);
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
        <span className={cn(pill, status === "open" && "bg-signal-soft text-foreground")}>
          {status === "open" ? "Open" : status === "out_of_scope" ? "Out of scope" : "Done"}
        </span>
        {level === 1 && <span className={pill}>User story</span>}
        {level === 0 && <span className={pill}>Feature</span>}
        {defaults > 0 && <span className={pill}>{defaults} answered by default</span>}
      </div>
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
          <Questions questions={read.questions} me={me} />
          <Rules rules={read.rules} nodeId={read.node.id} byId={byId} />
        </>
      )}
    </div>
  );
}

function Questions({ questions, me }: { questions: QuestionBody[]; me: string | null }) {
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
              <p className="mt-1.5 text-xs text-muted-foreground">
                {waits ? "Nobody has answered this yet." : `${who({ author: question.asked_by, given_by: "human" }, me)} · ${fmtDate(question.created_at)} · Remark, waits for the next grill pass`}
              </p>
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
                </>
              )
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
          <summary className="cursor-pointer py-1">{above.length} rules from above also hold here</summary>
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
