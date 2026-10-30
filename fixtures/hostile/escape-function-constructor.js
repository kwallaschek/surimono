// The Function constructor and indirect eval run in the same sandbox, so
// they must not see anything more than the script itself.
const viaFunction = Function("return typeof require + ',' + typeof process + ',' + typeof std")();
const viaEval = (0, eval)("typeof os + ',' + typeof scriptArgs");
viaFunction + "," + viaEval;
