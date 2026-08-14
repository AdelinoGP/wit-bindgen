//@ [lang]
//@ path = "stubs/world.ts"

import * as I from "../imports/test$resource_import_and_export$test";

// Straight pass-through of an owned imported handle. The generated wrapper
// must not release it: the caller gets it back.
export function toplevelExport(a: I.Thing): I.Thing {
  return a;
}
