// The 65,536th parameter gets index -1 from add_arg, unchecked: OP_get_arg 0xffff.
const params = new Array(65536).fill('{} = {}').join(',');
const GeneratorFunction = (function* () {}).constructor;
new GeneratorFunction(params, 'yield 7')().next();
