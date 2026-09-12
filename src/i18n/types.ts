// The shape every locale must have, derived from the English dictionary.
//
// `en/` is the source of truth for which keys exist. Each other locale is
// declared `satisfies SameShape<typeof en>`, which is deliberately *not*
// `satisfies typeof en`: that would demand the translations be assignable to
// the English literals, i.e. that they be the English text. What has to match
// is the key set and the fact that the values are strings.
//
// The payoff is that a missing key, a typo, or a namespace someone forgot to
// export is a `tsc` failure in `pnpm build` — which matters more than usual
// here, because this project has no frontend test runner at all.

/** A dictionary of nested namespaces whose leaves are all strings. */
export type Shape<T> = {
  [K in keyof T]: T[K] extends string ? string : Shape<T[K]>;
};

/** Every dotted leaf path of a dictionary, e.g. `"settings.language"`. */
export type KeyPath<T, Prefix extends string = ""> = {
  [K in keyof T & string]: T[K] extends string
    ? `${Prefix}${K}`
    : KeyPath<T[K], `${Prefix}${K}.`>;
}[keyof T & string];

/** Values substituted into a message's `{placeholders}`. */
export type Params = Record<string, string | number>;
