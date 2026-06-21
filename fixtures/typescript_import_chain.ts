// Fixture: cross-file import chain.
// Expected extraction: Import edges linking to ./utils and ./types.

import { helper } from './utils';
import type { Config } from './types';

export function init(cfg: Config): void {
    helper(cfg);
}
