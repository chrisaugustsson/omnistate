import type { ReactNode } from "react";
import { InPageTabs } from "./in-page-tabs";
import { PageHead } from "./page";
import { PagePath } from "./page-path";

/**
 * The head of an object's page (a feature, a track), under the bar that keeps
 * its part: the path (its section › the object), the title row, an optional
 * lede, and the object's own tabs. The tab's content follows it in the same
 * centred column (Page object); over a canvas tab the header is `wide` and
 * the canvas fills the window under it.
 */
export function ObjectHeader({
  path,
  mark,
  title,
  actions,
  lede,
  notice,
  tabsLabel,
  tabs,
  wide = false,
}: {
  /** The steps of the path, PathLinks; the last is the object, marked aria-current. */
  path: ReactNode[];
  /** A mark before the title (a feature's icon). */
  mark?: ReactNode;
  /** The object's name as the page title; leave out while the tab's own page still brings one. */
  title?: ReactNode;
  actions?: ReactNode;
  lede?: ReactNode;
  /** A quiet row under the lede, above the tabs: what comes next, or the track changing a feature. */
  notice?: ReactNode;
  /** The tabs' name ("Track", "Feature"). */
  tabsLabel: string;
  /** TabLinks. */
  tabs: ReactNode;
  /** Over a tab that is a canvas (Behaviour, Design): the full width of the window with 16px sides, tighter. */
  wide?: boolean;
}) {
  return (
    <div className={wide ? "w-full shrink-0 px-4 pt-3" : "mx-auto w-full max-w-(--page-max) px-4 pt-5 md:px-(--page-pad)"}>
      <PagePath>{path}</PagePath>
      {(title || actions) && <PageHead mark={mark} title={title} actions={actions} />}
      {lede && <p className="mt-1.5 max-w-(--main-max) text-em text-ink-2">{lede}</p>}
      {notice}
      <InPageTabs label={tabsLabel} className={wide ? "mt-2" : "mt-6"}>
        {tabs}
      </InPageTabs>
    </div>
  );
}
