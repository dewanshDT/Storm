import type { Env as FeedbackEnv } from "../src/env";

declare global {
  namespace Cloudflare {
    interface Env extends FeedbackEnv {}
  }
}

export {};
