import { afterEach } from "vitest";
import { forgetLoaded } from "@/lib/reloading";

// Each test starts with nothing kept from the one before (lib/reloading keeps what each page loaded last).
afterEach(() => {
  forgetLoaded();
});
