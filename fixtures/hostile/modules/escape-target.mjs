// The target of escape-dynamic-import.js: a real module a file-reading
// loader would find. Importing it from a hostile script must fail.
export default "LEAKED";
