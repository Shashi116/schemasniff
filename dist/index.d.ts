import type { InferSchemaReturn, InferSchemaOptions } from "./types";
export type { InferSchemaReturn, InferSchemaOptions, SchemaResult, SchemaError, SchemaErrorKind, ColumnMeta, InferredType, } from "./types";
export { isSchemaError, isSchemaResult } from "./types";
export declare function inferSchema(input: unknown, options?: InferSchemaOptions): Promise<InferSchemaReturn>;
//# sourceMappingURL=index.d.ts.map