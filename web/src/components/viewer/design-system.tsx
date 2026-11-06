import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Boxes, Search } from "lucide-react";
import { loadHeadHtml } from "@/api/docs";
import type { DocBody } from "@/api/generated";
import { createAssetLoader } from "@/design/assets";
import { createDesignRenderer, type DesignRenderer } from "@/design/renderer";
import { collectionsOf, componentLabel, object, readSystem, systemComponents, systemTokens, tokenSourceValue, tokenValues, type JsonObject, type SystemToken } from "@/design/system";
import { useReloading, type Reloading } from "@/lib/reloading";
import { cn } from "@/lib/utils";
import { useProjectEvents } from "@/live/context";
import { FloatingAlert, ToolbarSelect, ViewerToolbar } from "./chrome";

/** A design system is a catalog, not an HTML document containing its JSON payload. */
export function DesignSystemViewer({ workspaceId, projectId, docId, doc }: {
  workspaceId: string;
  projectId: string;
  docId: string;
  doc: Reloading<DocBody>;
}) {
  const head = useReloading(useCallback(async () => readSystem(await loadHeadHtml(workspaceId, projectId, docId)), [workspaceId, projectId, docId]));
  const system = head.value;
  const tokens = useMemo(() => systemTokens(system ?? {}), [system]);
  const components = useMemo(() => systemComponents(system ?? {}), [system]);
  const collections = useMemo(() => collectionsOf(system ?? {}), [system]);
  const [picked, pick] = useState<Record<string, string>>({});
  const choice = useMemo(() => Object.fromEntries(collections.flatMap(({ name, modes }) => modes.length ? [[name, modes.includes(picked[name]) ? picked[name] : modes[0]]] : [])), [collections, picked]);
  const values = useMemo(() => tokenValues(tokens, choice), [tokens, choice]);
  const mode = Object.values(choice)[0] ?? null;
  const [tab, setTab] = useState<"tokens" | "components">("tokens");
  const [search, setSearch] = useState("");
  const query = search.trim().toLowerCase();
  const shown = tokens.filter((token) => `${token.path} ${token.description}`.toLowerCase().includes(query));
  const shownComponents = components.filter((component) => componentLabel(component).toLowerCase().includes(query));

  useProjectEvents(projectId, (event) => {
    if (event.event === "doc.changed" && event.data.doc_id === docId) {
      head.reload();
      doc.reload();
    }
  });

  return (
    <main className="flex min-h-full min-w-0 flex-col">
      <ViewerToolbar workspaceId={workspaceId} projectId={projectId} docId={docId} doc={doc.value} icon={<Boxes aria-hidden />}>
        {collections.filter(({ modes }) => modes.length > 0).map(({ name, modes }) => <ToolbarSelect key={name} id={`system-mode-${name}`} label={name} value={choice[name]} onChange={(event) => pick((current) => ({ ...current, [name]: event.target.value }))}>
          {modes.map((mode) => <option key={mode} value={mode}>{mode}</option>)}
        </ToolbarSelect>)}
      </ViewerToolbar>
      <div className="mx-auto w-full max-w-[1440px] p-4 sm:p-8">
        <div className="mb-7 flex flex-wrap items-end justify-between gap-4">
          <div>
            <p className="mb-2 text-[11px] font-medium tracking-widest text-muted-foreground uppercase">Foundations & components</p>
            <h2 className="text-2xl font-semibold tracking-tight">Design system</h2>
            <p className="mt-2 text-sm text-muted-foreground">{tokens.length} tokens in {collections.length} collections · {components.length} components</p>
          </div>
          <div className="relative w-full sm:w-72">
            <Search aria-hidden className="absolute top-2.5 left-3 size-4 text-muted-foreground" />
            <input type="search" aria-label="Search design system" placeholder="Search names and descriptions…" value={search} onChange={(event) => setSearch(event.target.value)} className="h-9 w-full rounded-md border bg-background pr-3 pl-9 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring" />
          </div>
        </div>
        {head.failed && <FloatingAlert>The design system could not be loaded. Try refreshing the page.</FloatingAlert>}
        {!system && !head.failed && <p role="status" className="text-sm text-muted-foreground">Loading design system…</p>}
        {system && <>
          <nav aria-label="Design system sections" className="mb-6 flex flex-wrap gap-2 border-b pb-4">
            {([["tokens", "Tokens", tokens.length], ["components", "Components", components.length]] as const).map(([name, title, count]) => <button key={name} type="button" aria-pressed={tab === name} onClick={() => setTab(name)} className={cn("rounded-md px-3 py-2 text-sm transition-colors focus-visible:ring-2 focus-visible:ring-ring", tab === name ? "bg-primary text-primary-foreground" : "bg-muted/60 text-muted-foreground hover:bg-accent hover:text-foreground")}>
              {title}<span className="ml-2 text-xs opacity-70">{count}</span>
            </button>)}
          </nav>
          {tab === "components" ? <ComponentCatalog workspaceId={workspaceId} system={system} components={shownComponents} mode={mode} values={values} /> :
            <section aria-label="Design tokens" className="flex flex-col gap-8">
              {shown.length === 0 ? <p className="py-10 text-sm text-muted-foreground">No tokens match your search.</p> :
                collections.map(({ name, modes }) => {
                  const inCollection = shown.filter((token) => token.group === name);
                  if (inCollection.length === 0) return null;
                  return <section key={name} aria-label={`Collection ${name}`}>
                    <h3 className="mb-4 flex flex-wrap items-baseline gap-x-3 text-base font-semibold">{name}<span className="text-xs font-normal text-muted-foreground">{modes.length ? `modes: ${modes.join(", ")}` : "no modes"}</span></h3>
                    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
                      {inCollection.map((token) => <TokenCard key={token.path} token={token} value={values.get(token.path)} mode={choice[name] ?? null} />)}
                    </div>
                  </section>;
                })}
            </section>}
        </>}
      </div>
    </main>
  );
}

function TokenCard({ token, value, mode }: { token: SystemToken; value: unknown; mode: string | null }) {
  const string = typeof value === "string" ? value : "";
  const source = tokenSourceValue(token, mode);
  const reference = typeof source === "string" && source.startsWith("{") ? source : null;
  return <article className="min-w-0 overflow-hidden rounded-lg border bg-background">
    {token.type === "color" && <div aria-hidden className="h-20 border-b" style={{ backgroundColor: string }} />}
    {token.group === "radius" && <div aria-hidden className="flex h-20 items-center justify-center border-b bg-muted/30"><div className="h-12 w-20 border-2 border-foreground/40 bg-muted" style={{ borderRadius: string }} /></div>}
    {token.group === "spacing" && <div aria-hidden className="flex h-20 items-center px-4"><div className="h-3 max-w-full rounded-sm bg-foreground/40" style={{ width: string }} /></div>}
    {token.type === "shadow" && <div aria-hidden className="flex h-20 items-center justify-center bg-muted/30"><div className="h-10 w-20 rounded-md bg-background" style={{ boxShadow: shadowCss(value) }} /></div>}
    <div className="p-4">
      <h4 className="break-all font-mono text-xs font-medium">{token.path}</h4>
      {object(value) && token.type === "typography" ? <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
        {Object.entries(value).map(([key, field]) => <div key={key}><dt className="text-muted-foreground">{key}</dt><dd className="break-words">{String(field)}</dd></div>)}
      </dl> : <p className="mt-2 break-words font-mono text-xs text-muted-foreground">{token.type === "string" && typeof value === "string" ? JSON.stringify(value) : formatValue(value)}</p>}
      {token.scopes && token.scopes.length > 0 && <p className="mt-2 break-words text-xs text-muted-foreground">scopes: {token.scopes.join(", ")}</p>}
      {token.codeSyntax && Object.entries(token.codeSyntax).map(([platform, name]) => <p key={platform} className="mt-1 break-all text-xs text-muted-foreground">{platform}: {name}</p>)}
      {reference && <p className="mt-2 break-all text-xs text-muted-foreground">↳ {reference.slice(1, -1)}</p>}
      {token.description && <p className="mt-3 text-xs leading-relaxed text-muted-foreground">{token.description}</p>}
    </div>
  </article>;
}

function shadowCss(value: unknown): string | undefined {
  const shadows = Array.isArray(value) ? value : [value];
  if (!shadows.every(object)) return undefined;
  return shadows.map((shadow) => `${shadow.inset ? "inset " : ""}${shadow.offsetX} ${shadow.offsetY} ${shadow.blur} ${shadow.spread} ${shadow.color}`).join(", ");
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "Unresolved value";
  if (Array.isArray(value)) return value.map(formatValue).join(" · ");
  if (object(value)) return Object.entries(value).map(([key, part]) => `${key}: ${formatValue(part)}`).join(" · ");
  return String(value);
}

function ComponentCatalog({ workspaceId, system, components, mode, values }: {
  workspaceId: string;
  system: JsonObject;
  components: JsonObject[];
  mode: string | null;
  values: Map<string, unknown>;
}) {
  const [chosen, choose] = useState<string | null>(null);
  const selected = components.find((component) => component.id === chosen) ?? components[0];
  return <section aria-label="Design components">
    <h3 className="mb-4 text-base font-semibold">Components<span className="ml-2 font-normal text-muted-foreground">{components.length}</span></h3>
    {!selected ? <p className="py-10 text-sm text-muted-foreground">No components to show.</p> : <div className="grid items-start gap-6 lg:grid-cols-[240px_minmax(0,1fr)]">
      <div className="flex flex-col gap-1 rounded-lg border bg-background p-2">
        {components.map((component) => <button key={String(component.id)} type="button" aria-pressed={component.id === selected.id} onClick={() => choose(String(component.id))} className={cn("rounded-md px-3 py-2 text-left text-sm break-words focus-visible:ring-2 focus-visible:ring-ring", selected.id === component.id ? "bg-accent font-medium" : "text-muted-foreground hover:bg-accent/60")}>
          {componentLabel(component)}
        </button>)}
      </div>
      <div className="min-w-0 overflow-hidden rounded-lg border bg-background">
        <h4 className="border-b px-5 py-4 text-sm font-medium">{componentLabel(selected)}</h4>
        <ComponentPreview workspaceId={workspaceId} system={system} component={selected} mode={mode} values={values} />
        {object(selected.props) && Object.keys(selected.props).length > 0 && <div className="border-t p-5">
          <h5 className="mb-3 text-xs font-medium text-muted-foreground uppercase">Properties</h5>
          <dl className="grid gap-3 sm:grid-cols-2">{Object.entries(selected.props).map(([name, prop]) => <div key={name} className="min-w-0 text-sm"><dt className="font-medium">{name}</dt><dd className="mt-1 break-words text-xs text-muted-foreground">{object(prop) ? formatValue(prop.default) : formatValue(prop)}</dd>{object(prop) && Array.isArray(prop.values) && <dd className="mt-1 break-words text-xs text-muted-foreground">{prop.values.map(String).join(" · ")}</dd>}</div>)}</dl>
        </div>}
      </div>
    </div>}
  </section>;
}

function ComponentPreview({ workspaceId, system, component, mode, values }: {
  workspaceId: string;
  system: JsonObject;
  component: JsonObject;
  mode: string | null;
  values: Map<string, unknown>;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [renderer, setRenderer] = useState<DesignRenderer | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  useEffect(() => {
    if (!canvas.current) return;
    let cancelled = false;
    let made: DesignRenderer | null = null;
    void createDesignRenderer(canvas.current).then((created) => {
      if (cancelled) { created.destroy(); return; }
      made = created;
      setRenderer(created);
    }, () => { if (!cancelled) setFailure("The component preview could not start."); });
    return () => { cancelled = true; made?.destroy(); };
  }, []);
  const assets = useMemo(() => renderer ? createAssetLoader(renderer, workspaceId) : null, [renderer, workspaceId]);
  useEffect(() => {
    if (!renderer || !assets) return;
    let cancelled = false;
    const background = ["color.semantic.bg", "color.background", "color.bg", "color.surface"].map((path) => values.get(path)).find((value) => typeof value === "string") ?? (mode === "dark" ? "#18181b" : "#ffffff");
    const fixed = (size: unknown, minimum: number) => object(size) && typeof size.fixed === "number" ? Math.max(minimum, size.fixed + 64) : minimum;
    const page = {
      type: "page", id: "syspage1", name: "Component preview", children: [{
        type: "board", id: "sysbrd01", name: componentLabel(component), x: 0, y: 0,
        width: { fixed: fixed(component.width, 640) }, height: { fixed: fixed(component.height, 320) },
        fills: [{ type: "solid", color: background }],
        layout: { mode: "flex", direction: "column", alignItems: "center", justifyContent: "center", padding: { top: 32, right: 32, bottom: 32, left: 32 } },
        children: [{ type: "instance", id: "sysnstd1", name: componentLabel(component), component: component.id }],
      }],
    };
    setFailure(null);
    void assets.load(page, system).then((problems) => {
      if (cancelled) return;
      renderer.show(page, system);
      if (mode) renderer.setMode(mode);
      const standIns = renderer.fontsStoodIn().map((font) => `${font.family} ${font.weight} uses a fallback font.`);
      const warnings = [...problems, ...standIns];
      setFailure(warnings.length ? warnings.join(" ") : null);
    }).catch(() => { if (!cancelled) setFailure("This component could not be previewed."); });
    return () => { cancelled = true; };
  }, [renderer, assets, component, system, mode, values]);
  return <div className="relative h-80 min-w-0 sm:h-96">
    <canvas ref={canvas} aria-label={`Preview of ${componentLabel(component)}`} className="block h-full w-full" />
    {failure && <div role="alert" className="absolute inset-x-3 bottom-3 rounded-md border bg-background p-3 text-xs">{failure}</div>}
  </div>;
}
