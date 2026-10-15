import { expect, test } from "vitest";
import { healthResponse } from "./health";

test("the health response is 200 JSON with status ok", async () => {
  const response = healthResponse();
  expect(response.status).toBe(200);
  expect(response.headers.get("content-type")).toContain("application/json");
  expect(await response.json()).toEqual({ status: "ok" });
});
