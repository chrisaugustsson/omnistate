import { describe, expect, test } from "vitest";
import { collectionsOf, readSystem, systemTokens, tokenValues } from "./system";

describe("design-system token values", () => {
  test("inherits token types and resolves chained and composite aliases in the selected mode", () => {
    const tokens = systemTokens({
      $extensions: { source: { ignored: { $value: "metadata" } } },
      color: {
        $type: "color",
        surface: { $value: "#ffffff", $extensions: { omnistate: { modes: { dark: "#171717" } } } },
        background: { $value: "{color.surface}" },
        overlay: { $value: "{color.background}" },
      },
      spacing: { unit: { $type: "dimension", $value: "4px" } },
      shadow: { card: { $type: "shadow", $value: { color: "{color.overlay}", offsetX: "0px", offsetY: "{spacing.unit}", blur: "8px", spread: "0px" } } },
    });
    expect(tokens).toHaveLength(5);
    expect(tokens.find((token) => token.path === "color.overlay")?.type).toBe("color");
    expect(tokenValues(tokens, { color: "light" }).get("color.overlay")).toBe("#ffffff");
    expect(tokenValues(tokens, { color: "dark", spacing: "dark" }).get("shadow.card")).toEqual({ color: "#171717", offsetX: "0px", offsetY: "4px", blur: "8px", spread: "0px" });
  });

  test("collections take their own modes or the root list, and tokens resolve in their collection's choice", () => {
    const system = {
      $extensions: { omnistate: { modes: ["light", "dark"] } },
      color: { $type: "color", ink: { $value: "#000000", $extensions: { omnistate: { modes: { dark: "#ffffff" } } } } },
      density: {
        $extensions: { omnistate: { modes: ["comfortable", "compact"] } },
        $type: "dimension",
        gap: { $value: "16px", $extensions: { omnistate: { modes: { compact: "12px" } } } },
      },
      state: {
        $extensions: { omnistate: { modes: [] } },
        count: { $type: "number", $value: 2, $extensions: { omnistate: { scopes: ["content", "opacity"], codeSyntax: { web: "count", ios: "Count" } } } },
      },
    };
    expect(collectionsOf(system)).toEqual([
      { name: "color", modes: ["light", "dark"] },
      { name: "density", modes: ["comfortable", "compact"] },
      { name: "state", modes: [] },
    ]);
    const tokens = systemTokens(system);
    const values = tokenValues(tokens, { color: "light", density: "compact" });
    expect(values.get("color.ink")).toBe("#000000");
    expect(values.get("density.gap")).toBe("12px");
    expect(values.get("state.count")).toBe(2);
    expect(tokens.find((token) => token.path === "state.count")).toMatchObject({ scopes: ["content", "opacity"], codeSyntax: { web: "count", ios: "Count" } });
  });

  test("an unresolved or cyclic reference does not stop valid tokens from being shown", () => {
    const tokens = systemTokens({ color: { $type: "color", good: { $value: "#123456" }, missing: { $value: "{color.absent}" }, a: { $value: "{color.b}" }, b: { $value: "{color.a}" } } });
    const values = tokenValues(tokens, {});
    expect(values.get("color.good")).toBe("#123456");
    expect(values.get("color.missing")).toBeNull();
    expect(values.get("color.a")).toBeNull();
    expect(values.get("color.b")).toBeNull();
  });

  test("an empty head is an empty catalog and a non-object payload is rejected", () => {
    expect(systemTokens(readSystem(""))).toEqual([]);
    expect(() => readSystem("[]")).toThrow();
  });
});
