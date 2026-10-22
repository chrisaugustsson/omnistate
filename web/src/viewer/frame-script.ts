// The viewer's own script: the only script that runs in the doc frame
// (document.ts puts it inline, allowed by its hash). It is plain browser
// JavaScript kept in a string, because the frame's document carries it as
// text; it must not contain a backtick or a `${`. `PARENT_ORIGIN` is declared
// before it by document.ts, and `mermaid` is the global of the Mermaid script
// that precedes it.
//
// The frame and the page talk only through postMessage (the protocol is
// described in web/src/doc-viewer-frame.test.ts):
//
// - to the page: `viewer.ready` once the script runs, and `viewer.select` with
//   the node ID of the innermost block that the reviewer clicked;
// - from the page: `viewer.show` with the doc's `html`, the `pins`
//   (`[{ node, count }]`), the `selected` node, the live `claims` and, for a
//   change set's review, the `changes` (`[{ node, change }]`, `change` being
//   `added`, `changed` or `removed`), which the frame marks block by block.
export const FRAME_SCRIPT = String.raw`(function () {
  "use strict";
  var page = window.parent;
  var doc = document.getElementById("doc");
  var pinLayer = document.getElementById("pins");
  var shown = { html: null, pins: [], selected: null, claims: [], changes: [] };
  var hovered = null;
  var drawn = {};
  var counter = 0;
  var configured = false;

  function post(message) {
    page.postMessage(message, PARENT_ORIGIN);
  }

  function blockOf(target) {
    return target instanceof Element ? target.closest("[data-node-id]") : null;
  }

  function find(node) {
    var blocks = doc.querySelectorAll("[data-node-id]");
    for (var i = 0; i < blocks.length; i += 1) {
      if (blocks[i].getAttribute("data-node-id") === node) return blocks[i];
    }
    return null;
  }

  // The doc's HTML is already cleaned by the API; this keeps only what a doc
  // may hold, whatever the page sends, and the policy blocks the rest.
  function parse(html) {
    var parsed = new DOMParser().parseFromString(html, "text/html");
    var banned = "script, style, link, meta, base, iframe, object, embed, form";
    Array.prototype.forEach.call(parsed.querySelectorAll(banned), function (element) {
      element.remove();
    });
    Array.prototype.forEach.call(parsed.querySelectorAll("*"), function (element) {
      Array.prototype.forEach.call(Array.from(element.attributes), function (attribute) {
        var name = attribute.name.toLowerCase();
        var value = attribute.value.replace(/[\s\u0000-\u001f]/g, "").toLowerCase();
        if (name.indexOf("on") === 0 || ((name === "href" || name === "src" || name === "xlink:href") && value.indexOf("javascript:") === 0)) {
          element.removeAttribute(attribute.name);
        }
      });
    });
    return parsed.body;
  }

  function place(pre, svg) {
    if (!pre.isConnected) return;
    var figure = document.createElement("div");
    figure.className = "omni-mermaid";
    figure.innerHTML = svg;
    pre.replaceWith(figure);
    layoutPins();
  }

  // Mermaid figures: the diagram's text is drawn as an SVG in its place.
  function drawDiagrams() {
    var engine = window.mermaid && (window.mermaid.default || window.mermaid);
    if (!engine) return;
    if (!configured) {
      engine.initialize({ startOnLoad: false, securityLevel: "strict", theme: "neutral" });
      configured = true;
    }
    Array.prototype.forEach.call(doc.querySelectorAll("pre.mermaid"), function (pre) {
      var source = pre.textContent || "";
      if (!Object.prototype.hasOwnProperty.call(drawn, source)) {
        counter += 1;
        drawn[source] = engine.render("omni-diagram-" + counter, source).then(
          function (result) { return result.svg; },
          function () { return null; }
        );
      }
      drawn[source].then(function (svg) {
        if (svg) place(pre, svg);
      });
    });
  }

  function setHover(block) {
    var current = hovered && find(hovered);
    if (current) current.classList.remove("is-hover");
    hovered = block ? block.getAttribute("data-node-id") : null;
    if (block) block.classList.add("is-hover");
  }

  function layoutPins() {
    pinLayer.replaceChildren();
    var left = doc.getBoundingClientRect().right + window.scrollX + 8;
    var placed = [];
    shown.pins.forEach(function (pin) {
      var block = pin && find(pin.node);
      if (!block || !(pin.count > 0)) return;
      var top = block.getBoundingClientRect().top + window.scrollY;
      var slot = 0;
      while (placed.some(function (other) { return other.slot === slot && Math.abs(other.top - top) < 26; })) slot += 1;
      placed.push({ top: top, slot: slot });
      var button = document.createElement("button");
      button.type = "button";
      button.className = "omni-pin";
      button.setAttribute("data-pin-for", pin.node);
      button.setAttribute("aria-label", pin.count + (pin.count === 1 ? " comment" : " comments"));
      button.textContent = String(pin.count);
      button.style.top = top + "px";
      button.style.left = left + slot * 28 + "px";
      pinLayer.appendChild(button);
    });
  }

  function markBlocks() {
    Array.prototype.forEach.call(doc.querySelectorAll(".is-selected, .is-claimed"), function (block) {
      block.classList.remove("is-selected", "is-claimed");
      block.removeAttribute("title");
    });
    Array.prototype.forEach.call(doc.querySelectorAll(".is-added, .is-changed, .is-removed"), function (block) {
      block.classList.remove("is-added", "is-changed", "is-removed");
    });
    shown.changes.forEach(function (entry) {
      var block = entry && find(entry.node);
      if (block && (entry.change === "added" || entry.change === "changed" || entry.change === "removed")) {
        block.classList.add("is-" + entry.change);
      }
    });
    var selected = shown.selected && find(shown.selected);
    if (selected) selected.classList.add("is-selected");
    shown.claims.forEach(function (claim) {
      var block = claim && find(claim.node);
      if (!block || !claim.agent) return;
      block.classList.add("is-claimed");
      block.setAttribute("title", "Claimed by " + claim.agent.run + "/" + claim.agent.role);
    });
    layoutPins();
  }

  function show(message) {
    shown.pins = Array.isArray(message.pins) ? message.pins : [];
    shown.selected = typeof message.selected === "string" ? message.selected : null;
    shown.claims = Array.isArray(message.claims) ? message.claims : [];
    shown.changes = Array.isArray(message.changes) ? message.changes : [];
    if (typeof message.html === "string" && message.html !== shown.html) {
      shown.html = message.html;
      doc.replaceChildren.apply(doc, Array.from(parse(message.html).childNodes));
      drawDiagrams();
      var again = hovered && find(hovered);
      if (again) again.classList.add("is-hover");
    }
    markBlocks();
  }

  window.addEventListener("message", function (event) {
    if (event.source !== page || event.origin !== PARENT_ORIGIN) return;
    var message = event.data;
    if (message && message.type === "viewer.show") show(message);
  });

  document.addEventListener("mouseover", function (event) {
    setHover(blockOf(event.target));
  });
  document.addEventListener("mouseout", function (event) {
    if (!event.relatedTarget) setHover(null);
  });
  document.documentElement.addEventListener("mouseleave", function () {
    setHover(null);
  });

  document.addEventListener("click", function (event) {
    var target = event.target instanceof Element ? event.target : null;
    if (target && target.closest("a")) event.preventDefault();
    var pin = target && target.closest("[data-pin-for]");
    var block = pin ? find(pin.getAttribute("data-pin-for")) : blockOf(target);
    if (block) post({ type: "viewer.select", node: block.getAttribute("data-node-id") });
  });

  var frameRequested = false;
  function relayout() {
    if (frameRequested) return;
    frameRequested = true;
    window.requestAnimationFrame(function () {
      frameRequested = false;
      layoutPins();
    });
  }
  window.addEventListener("resize", relayout);
  if (window.ResizeObserver) new ResizeObserver(relayout).observe(doc);

  post({ type: "viewer.ready" });
})();`;
