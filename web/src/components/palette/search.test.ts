// US-125: the palette's search over the project home (an allowed exception to
// the web seam): feature names, doc titles and thread questions, nothing else.
import { describe, expect, test } from "vitest";
import type { HomeResponse } from "@/api/generated";
import { buildIndex, recentPlaces, search } from "@/components/palette/search";

const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const CHECKOUT = "0192d7a0-4444-7000-8000-000000000001";
const WALLETS = "0192d7a0-4444-7000-8000-000000000002";
const GONE = "0192d7a0-4444-7000-8000-000000000009";
const APP_DOC = "0192d7a0-5555-7000-8000-000000000001";
const CHECKOUT_DOC = "0192d7a0-5555-7000-8000-000000000002";
const GONE_DOC = "0192d7a0-5555-7000-8000-000000000009";
const CHECKOUT_THREAD = "0192d7a0-6666-7000-8000-000000000001";
const APP_THREAD = "0192d7a0-6666-7000-8000-000000000002";
const AT = "2026-10-21T08:00:00Z";

let ids = 0;
const newId = () => `0192d7a0-9999-7000-8000-${String(++ids).padStart(12, "0")}`;

function feature(name: string, fields: { id?: string; description?: string; lastActivity?: string | null; createdAt?: string } = {}) {
  return {
    feature: {
      id: fields.id ?? newId(),
      project_id: PROJECT_ID,
      name,
      description: fields.description ?? "",
      created_at: fields.createdAt ?? AT,
    },
    docs: 0,
    open_changesets: 0,
    waiting: 0,
    last_activity: fields.lastActivity ?? null,
  };
}

function doc(title: string, fields: { id?: string; feature?: string; pillar?: string; updatedAt?: string; createdAt?: string } = {}) {
  return {
    id: fields.id ?? newId(),
    project_id: PROJECT_ID,
    feature_id: fields.feature ?? null,
    kind: "html",
    pillar: fields.pillar ?? "spec",
    title,
    created_at: fields.createdAt ?? AT,
    updated_at: fields.updatedAt ?? AT,
    waiting: 0,
    drafts: [],
    claims: [],
  };
}

function thread(question: string, fields: { id?: string; feature?: string; lastPostAt?: string | null; createdAt?: string } = {}) {
  return {
    id: fields.id ?? newId(),
    project_id: PROJECT_ID,
    feature_id: fields.feature ?? null,
    question,
    sealed: false,
    conclusion: null,
    created_at: fields.createdAt ?? AT,
    post_count: fields.lastPostAt ? 1 : 0,
    last_post_at: fields.lastPostAt ?? null,
  };
}

function home(fields: {
  features?: ReturnType<typeof feature>[];
  docs?: ReturnType<typeof doc>[];
  threads?: ReturnType<typeof thread>[];
}): HomeResponse {
  return {
    project_id: PROJECT_ID,
    app: { last_activity: null },
    features: fields.features ?? [],
    docs: fields.docs ?? [],
    changesets: [],
    threads: fields.threads ?? [],
    waiting: { decisions: [], changesets: [], comments: [], questions: [] },
  } as HomeResponse;
}

const names = (group: { items: { name: string }[] }) => group.items.map((item) => item.name);

describe("the palette's search", () => {
  test("finds feature names, doc titles and thread questions containing the trimmed query, ignoring case, and nothing by description or scope", () => {
    const index = buildIndex(
      home({
        features: [
          feature("Quick checkout", { id: CHECKOUT, description: "Pay in one step." }),
          feature("Wallets", { id: WALLETS, description: "Checkout with Apple Pay and Google Pay." }),
        ],
        docs: [doc("The checkout PRD"), doc("Payment screens", { feature: CHECKOUT, pillar: "ui" })],
        threads: [thread("Does checkout need a login?"), thread("Which wallets?", { feature: CHECKOUT })],
      }),
    );

    const results = search(index, "  CHECKOUT ");

    expect(names(results.features)).toEqual(["Quick checkout"]);
    expect(names(results.docs)).toEqual(["The checkout PRD"]);
    expect(names(results.threads)).toEqual(["Does checkout need a login?"]);
  });

  test("puts what starts with the query first, then the other matches, each part newest first, ties by name", () => {
    const index = buildIndex(
      home({
        docs: [
          doc("One-step checkout", { updatedAt: "2026-10-21T11:00:00Z" }),
          doc("checkout screens", { updatedAt: "2026-10-21T09:00:00Z" }),
          doc("Guest checkout", { updatedAt: "2026-10-21T11:00:00Z" }),
          doc("Fast checkout", { updatedAt: "2026-10-21T12:00:00Z" }),
          doc("Checkout PRD", { updatedAt: "2026-10-21T10:00:00Z" }),
          doc("Express checkout", { updatedAt: "2026-10-21T11:00:00Z" }),
        ],
      }),
    );

    expect(names(search(index, " Checkout").docs)).toEqual([
      "Checkout PRD",
      "checkout screens",
      "Fast checkout",
      "Express checkout",
      "Guest checkout",
      "One-step checkout",
    ]);
  });

  test("orders a feature by its last activity and a thread by its newest post, each by its creation while it has none, and a doc by its last write", () => {
    const index = buildIndex(
      home({
        features: [
          feature("Cart limits", { lastActivity: "2026-10-21T10:00:00Z", createdAt: "2026-10-21T08:00:00Z" }),
          feature("Cart sharing", { lastActivity: "2026-10-21T09:00:00Z", createdAt: "2026-10-21T12:00:00Z" }),
          feature("Cart expiry", { lastActivity: null, createdAt: "2026-10-21T11:00:00Z" }),
        ],
        docs: [
          doc("Cart flow", { updatedAt: "2026-10-21T09:00:00Z", createdAt: "2026-10-21T12:00:00Z" }),
          doc("Cart PRD", { updatedAt: "2026-10-21T10:00:00Z", createdAt: "2026-10-21T06:00:00Z" }),
        ],
        threads: [
          thread("Cart limit per user?", { lastPostAt: "2026-10-21T10:00:00Z", createdAt: "2026-10-21T07:00:00Z" }),
          thread("Cart sharing by link?", { lastPostAt: "2026-10-21T09:00:00Z", createdAt: "2026-10-21T12:00:00Z" }),
          thread("Cart expiry after a day?", { lastPostAt: null, createdAt: "2026-10-21T11:00:00Z" }),
        ],
      }),
    );

    const results = search(index, "cart");

    expect(names(results.features)).toEqual(["Cart expiry", "Cart limits", "Cart sharing"]);
    expect(names(results.docs)).toEqual(["Cart PRD", "Cart flow"]);
    expect(names(results.threads)).toEqual(["Cart expiry after a day?", "Cart limit per user?", "Cart sharing by link?"]);
  });

  test("shows at most 5 features, 6 docs and 3 threads, and counts every match", () => {
    const index = buildIndex(
      home({
        features: ["Plan A", "Plan B", "Plan C", "Plan D", "Plan E", "Plan F"].map((name) => feature(name)),
        docs: ["Plan 1", "Plan 2", "Plan 3", "Plan 4", "Plan 5", "Plan 6", "Plan 7"].map((title) => doc(title)),
        threads: ["Plan a?", "Plan b?", "Plan c?", "Plan d?"].map((question) => thread(question)),
      }),
    );

    const results = search(index, "plan");

    expect(names(results.features)).toEqual(["Plan A", "Plan B", "Plan C", "Plan D", "Plan E"]);
    expect(results.features.total).toBe(6);
    expect(names(results.docs)).toEqual(["Plan 1", "Plan 2", "Plan 3", "Plan 4", "Plan 5", "Plan 6"]);
    expect(results.docs.total).toBe(7);
    expect(names(results.threads)).toEqual(["Plan a?", "Plan b?", "Plan c?"]);
    expect(results.threads.total).toBe(4);
  });

  test("one search over 300 features, 2,000 docs and 500 threads takes under 50 ms", () => {
    const index = buildIndex(
      home({
        features: Array.from({ length: 300 }, (_, n) => feature(`Checkout step ${n}`, { lastActivity: AT })),
        docs: Array.from({ length: 2000 }, (_, n) => doc(`Notes on checkout ${n}`)),
        threads: Array.from({ length: 500 }, (_, n) => thread(`Should checkout ${n} ask twice?`)),
      }),
    );

    const start = performance.now();
    const results = search(index, "ch");
    const took = performance.now() - start;

    expect([results.features.total, results.docs.total, results.threads.total]).toEqual([300, 2000, 500]);
    expect(took).toBeLessThan(50);
  });
});

describe("the palette's recent places", () => {
  test("are the 6 newest visits the project still has, a doc with its scope and pillar, a thread with its scope, a scope page with none", () => {
    const index = buildIndex(
      home({
        features: [feature("Checkout", { id: CHECKOUT }), feature("Wallets", { id: WALLETS })],
        docs: [
          doc("Returns PRD", { id: APP_DOC }),
          doc("Pay in one step", { id: CHECKOUT_DOC, feature: CHECKOUT, pillar: "ui" }),
        ],
        threads: [thread("Which wallets?", { id: CHECKOUT_THREAD, feature: CHECKOUT }), thread("Which hero?", { id: APP_THREAD })],
      }),
    );
    const visits = [
      { scope: CHECKOUT, doc: CHECKOUT_DOC },
      { scope: GONE },
      { scope: CHECKOUT, thread: CHECKOUT_THREAD },
      { scope: "app" },
      { scope: "app", doc: GONE_DOC },
      { scope: WALLETS },
      { scope: "app", thread: APP_THREAD },
      { scope: "app", doc: APP_DOC },
      { scope: CHECKOUT },
    ].map((visit, n) => ({ ...visit, at: Date.parse("2026-10-21T12:00:00Z") - n * 60_000 }));

    expect(recentPlaces(index, visits).map(({ name, context }) => ({ name, context }))).toEqual([
      { name: "Pay in one step", context: "Checkout · UI" },
      { name: "Which wallets?", context: "Checkout" },
      { name: "App", context: "" },
      { name: "Wallets", context: "" },
      { name: "Which hero?", context: "App" },
      { name: "Returns PRD", context: "App · Spec" },
    ]);
  });
});
