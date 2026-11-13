// The viewer's own script, which runs in the doc frame beside the doc's own
// scripts (US-109)
// (document.ts puts it inline). It is plain browser
// JavaScript kept in a string, because the frame's document carries it as
// text; it must not contain a backtick or a `${`. `PARENT_ORIGIN` is declared
// before it by document.ts.
//
// The frame and the page talk only through postMessage (the protocol is
// described in web/src/doc-viewer-frame.test.ts):
//
// - to the page: `viewer.ready` once the script runs, and `viewer.select` with
//   the node ID of the innermost block that the reviewer clicked;
// - from the page: `viewer.show` with the doc's `html`, the `pins`
//   (`[{ node, count }]`), the `selected` node and `reveal`, a node ID that the
//   frame scrolls into view when it is not the one it revealed last. When the
//   doc's `html` changes, the frame draws it again and puts the scroll
//   position back where it was.
//
// A frame in a browser tab that loads in the background is 0 wide and 0 tall
// until the tab is shown, and nothing can be measured in it. The doc's HTML
// goes in at once, but its scripts wait until the frame has a size.
export const FRAME_SCRIPT = String.raw`(function () {
  "use strict";
  var page = window.parent;
  var doc = document.getElementById("doc");
  var pinLayer = document.getElementById("pins");
  var shown = { html: null, pins: [], selected: null, revealed: null };
  var hovered = null;
  var waiting = false;

  function post(message) {
    page.postMessage(message, PARENT_ORIGIN);
  }

  function blockOf(target) {
    return target instanceof Element ? target.closest("[data-omni-id]") : null;
  }

  function find(node) {
    var blocks = doc.querySelectorAll("[data-omni-id]");
    for (var i = 0; i < blocks.length; i += 1) {
      if (blocks[i].getAttribute("data-omni-id") === node) return blocks[i];
    }
    return null;
  }

  // The doc is free-form HTML (docs/formats/html-docs.md): its styles, scripts
  // and event handlers stay, and run inside this sandboxed frame, which has
  // no origin of its own and a policy that blocks every request. Only what
  // could navigate or restyle the frame itself is left out.
  function parse(html) {
    var parsed = new DOMParser().parseFromString(html, "text/html");
    var banned = "base, link, iframe, object, embed, meta[http-equiv]";
    Array.prototype.forEach.call(parsed.querySelectorAll(banned), function (element) {
      element.remove();
    });
    return parsed.body;
  }

  // A script that a parser built does not run when it is moved into the
  // frame's document, so each one is made again.
  function revive(root) {
    Array.prototype.forEach.call(root.querySelectorAll("script"), function (old) {
      var script = document.createElement("script");
      Array.prototype.forEach.call(Array.from(old.attributes), function (attribute) {
        if (attribute.name.toLowerCase() !== "src") script.setAttribute(attribute.name, attribute.value);
      });
      script.textContent = old.textContent;
      old.replaceWith(script);
    });
  }

  // Runs the doc's scripts, once the frame has a size.
  function start() {
    if (!waiting || !(window.innerWidth > 0)) return;
    waiting = false;
    revive(doc);
  }

  function setHover(block) {
    var current = hovered && find(hovered);
    if (current) current.classList.remove("is-hover");
    hovered = block ? block.getAttribute("data-omni-id") : null;
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
    Array.prototype.forEach.call(doc.querySelectorAll(".is-selected"), function (block) {
      block.classList.remove("is-selected");
    });
    var selected = shown.selected && find(shown.selected);
    if (selected) selected.classList.add("is-selected");
    layoutPins();
  }

  function show(message) {
    shown.pins = Array.isArray(message.pins) ? message.pins : [];
    shown.selected = typeof message.selected === "string" ? message.selected : null;
    if (typeof message.html === "string" && message.html !== shown.html) {
      var scrollX = window.scrollX;
      var scrollY = window.scrollY;
      shown.html = message.html;
      doc.replaceChildren.apply(doc, Array.from(parse(message.html).childNodes));
      waiting = true;
      start();
      var again = hovered && find(hovered);
      if (again) again.classList.add("is-hover");
      window.scrollTo(scrollX, scrollY);
    }
    markBlocks();
    if (typeof message.reveal !== "string") shown.revealed = null;
    else if (message.reveal !== shown.revealed) {
      var target = find(message.reveal);
      if (target) {
        shown.revealed = message.reveal;
        target.scrollIntoView({ block: "center" });
      }
    }
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
    if (block) post({ type: "viewer.select", node: block.getAttribute("data-omni-id") });
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
  window.addEventListener("resize", start);
  if (window.ResizeObserver) new ResizeObserver(relayout).observe(doc);

  post({ type: "viewer.ready" });
})();`;
