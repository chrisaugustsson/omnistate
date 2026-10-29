import { z } from "zod";
import { callApi } from "./client";
import {
  ActivityEntrySchema,
  ActivityListResponseSchema,
  BoardBodySchema,
  ComponentBodySchema,
  TitleChangeSchema,
  TokensChangeSchema,
  type ActivityListResponse,
} from "./generated";

const base = (workspaceId: string, projectId: string, docId: string) =>
  `/api/v1/w/${workspaceId}/projects/${projectId}/docs/${docId}`;

/** One thing a write changed: an element of an HTML doc, a board of a design or a component of a design system. */
export type ActivityItem =
  | { kind: "node"; node: string | null; before: string | null; after: string | null }
  | { kind: "board"; board: string; before_at: number | null; after_at: number | null }
  | { kind: "component"; component: string; before_at: number | null; after_at: number | null };

// The generated schema of an item is a union whose first variant (all keys
// optional) accepts every item and drops the keys of a board or a component,
// so the items are read here, the keyed variants first.
const ItemSchema = z.union([
  z
    .object({ board: z.string(), before_at: z.number().nullish(), after_at: z.number().nullish() })
    .transform((item): ActivityItem => ({ kind: "board", board: item.board, before_at: item.before_at ?? null, after_at: item.after_at ?? null })),
  z
    .object({ component: z.string(), before_at: z.number().nullish(), after_at: z.number().nullish() })
    .transform(
      (item): ActivityItem => ({ kind: "component", component: item.component, before_at: item.before_at ?? null, after_at: item.after_at ?? null }),
    ),
  z
    .object({ node: z.string().nullish(), before: z.string().nullish(), after: z.string().nullish() })
    .transform((item): ActivityItem => ({ kind: "node", node: item.node ?? null, before: item.before ?? null, after: item.after ?? null })),
]);

const DetailSchema = ActivityEntrySchema.extend({
  items: z.array(ItemSchema),
  title: TitleChangeSchema.nullish(),
  tokens: TokensChangeSchema.nullish(),
});

/** What one write changed, with its entry. */
export type ActivityDetail = z.infer<typeof DetailSchema>;

/** A page of the doc's writes, newest first; `before` is the `next_before` of the page before. */
export function loadActivity(
  workspaceId: string,
  projectId: string,
  docId: string,
  before?: number,
): Promise<ActivityListResponse> {
  const query = before === undefined ? "" : `?before=${before}`;
  return callApi("GET", `${base(workspaceId, projectId, docId)}/activity${query}`, {
    response: ActivityListResponseSchema,
  });
}

/** One write to the doc and what it changed. */
export function loadActivityEntry(
  workspaceId: string,
  projectId: string,
  docId: string,
  seq: number,
): Promise<ActivityDetail> {
  return callApi("GET", `${base(workspaceId, projectId, docId)}/activity/${seq}`, { response: DetailSchema });
}

/** A board of a design as it was right after write `at`, in the tag form. */
export async function loadBoardAt(
  workspaceId: string,
  projectId: string,
  docId: string,
  boardId: string,
  at: number,
): Promise<string> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId, docId)}/boards/${boardId}?at=${at}`, {
    response: BoardBodySchema,
  });
  return loaded.tags;
}

/** A component of the design system as it was right after write `at`, as a JSON node. */
export async function loadComponentAt(
  workspaceId: string,
  projectId: string,
  componentId: string,
  at: number,
): Promise<Record<string, unknown>> {
  const loaded = await callApi(
    "GET",
    `/api/v1/w/${workspaceId}/projects/${projectId}/components/${componentId}?at=${at}`,
    { response: ComponentBodySchema },
  );
  return loaded.node;
}
