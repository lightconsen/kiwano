// The mock's row ids. The provider-add and the key-add path draw from one
// counter, as the two tables of the real store draw from one database.

/** The sequence the database would hand back: `addProvider` and `addApiKey`
    both draw from it, which is why it cannot live in either of them. */
let idSeq = 100;

/** The next row id. */
export const nextId = (): number => ++idSeq;
