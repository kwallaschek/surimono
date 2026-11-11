//! `Math` and `**` give the same bits on every platform (P2-RT-07, engine
//! patch 38): QuickJS calls CORE-MATH's correctly rounded functions instead of
//! the platform's libm.
//!
//! Outside truth:
//! - [`CORRECTLY_ROUNDED`]: the exact value rounded to nearest (IEEE 754-2019
//!   §9.2 recommends correctly rounded functions), computed with mpmath 1.4.1
//!   at 300 bits; nothing here was taken from our output. The inputs come
//!   from a fixed sweep, picked where glibc 2.39's or fdlibm's (the `libm`
//!   crate's, musl's) result differs, so a build on the platform's libm fails
//!   here, plus plain ones. Each comment says which of glibc 2.39 (QuickJS on
//!   x86_64 Linux before the patch), fdlibm and Chrome 153 gave other bits,
//!   recorded on 10 Nov 2026.
//! - [`SPEC`]: values ECMA-262 (§6.1.6.1.3 Number::exponentiate, §21.3.2
//!   Function Properties of the Math Object) requires exactly, and exact
//!   results.

use surimono_core::{Limits, Sandbox};

/// `(function, argument bits, correctly rounded result bits)`.
#[rustfmt::skip]
const CORRECTLY_ROUNDED: &[(&str, &[u64], u64)] = &[
    // sin(173112.48707695375) = -0.9654295264820164; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("sin", &[0x410521c3e5889a18], 0xbfeee4cc765aa827),
    // sin(352.6056676708322) = 0.6796537264679922; glibc 2.39 differs, Chrome 153 agrees
    ("sin", &[0x407609b0d0956780], 0x3fe5bfb92bf91b15),
    // sin(903912.0568052777) = 0.4368949188379843; fdlibm differs, Chrome 153 agrees
    ("sin", &[0x412b95d01d1594d4], 0x3fdbf6161b0ca765),
    // sin(490368.90883456497) = -0.7533414086308583; fdlibm differs, Chrome 153 agrees
    ("sin", &[0x411dee03a2a58738], 0xbfe81b5f711958f4),
    // sin(1.0) = 0.8414709848078965; Chrome 153 agrees
    ("sin", &[0x3ff0000000000000], 0x3feaed548f090cee),
    // sin(-1.0) = -0.8414709848078965; Chrome 153 agrees
    ("sin", &[0xbff0000000000000], 0xbfeaed548f090cee),
    // cos(620587.997663944) = -0.6006627923889357; fdlibm differs, Chrome 153 agrees
    ("cos", &[0x4122f057fecdcef8], 0xbfe338a12d278129),
    // cos(-966731.4598933399) = 0.8426929535534801; fdlibm differs, Chrome 153 agrees
    ("cos", &[0xc12d8096eb7723cd], 0x3feaf75736829e77),
    // cos(1.0) = 0.5403023058681398; Chrome 153 agrees
    ("cos", &[0x3ff0000000000000], 0x3fe14a280fb5068c),
    // cos(-1.0) = 0.5403023058681398; Chrome 153 agrees
    ("cos", &[0xbff0000000000000], 0x3fe14a280fb5068c),
    // tan(-220.9153658812943) = -1.570780058405856; glibc 2.39 differs, Chrome 153 agrees
    ("tan", &[0xc06b9d4aad638110], 0xbff921ea4540fe73),
    // tan(1e+22) = -1.6287782256068988; fdlibm differs, Chrome 153 agrees
    ("tan", &[0x4480f0cf064dd592], 0xbffa0f79c1b6b257),
    // tan(-159.57891754780394) = 0.7481370984024315; fdlibm differs, Chrome 153 agrees
    ("tan", &[0xc063f2867e17dcc0], 0x3fe7f0bd36520245),
    // tan(1.0) = 1.5574077246549023; Chrome 153 agrees
    ("tan", &[0x3ff0000000000000], 0x3ff8eb245cbee3a6),
    // tan(-1.0) = -1.5574077246549023; Chrome 153 agrees
    ("tan", &[0xbff0000000000000], 0xbff8eb245cbee3a6),
    // asin(-0.8906623336382156) = -1.0987998470547; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("asin", &[0xbfec804e4b582a68], 0xbff194af25ff3221),
    // asin(-0.9204030340210987) = -1.1691100918262756; fdlibm differs, Chrome 153 agrees
    ("asin", &[0xbfed73f11048503c], 0xbff2b4acc89d141d),
    // asin(0.6410734986214186) = 0.6958961825734212; fdlibm differs, Chrome 153 agrees
    ("asin", &[0x3fe483ac91dd2952], 0x3fe644c812320d05),
    // asin(1.0) = 1.5707963267948966; Chrome 153 agrees
    ("asin", &[0x3ff0000000000000], 0x3ff921fb54442d18),
    // asin(-1.0) = -1.5707963267948966; Chrome 153 agrees
    ("asin", &[0xbff0000000000000], 0xbff921fb54442d18),
    // acos(-0.6184663578684877) = 2.2375858602789136; fdlibm differs, Chrome 153 agrees
    ("acos", &[0xbfe3ca79f597159a], 0x4001e6936a5f1ea3),
    // acos(-0.6970149663577212) = 2.3420224559924088; fdlibm differs, Chrome 153 agrees
    ("acos", &[0xbfe64df254aa86c8], 0x4002bc7644f7e145),
    // acos(-1.0) = 3.141592653589793; Chrome 153 agrees
    ("acos", &[0xbff0000000000000], 0x400921fb54442d18),
    // acos(0.5) = 1.0471975511965979; Chrome 153 agrees
    ("acos", &[0x3fe0000000000000], 0x3ff0c152382d7366),
    // atan(3.141592653589793) = 1.2626272556789118; fdlibm differs, Chrome 153 agrees
    ("atan", &[0x400921fb54442d18], 0x3ff433b8a322ddd3),
    // atan(1.01) = 0.7903732467283023; fdlibm differs, Chrome 153 agrees
    ("atan", &[0x3ff028f5c28f5c29], 0x3fe94abcd5ca9acf),
    // atan(1.0) = 0.7853981633974483; Chrome 153 agrees
    ("atan", &[0x3ff0000000000000], 0x3fe921fb54442d18),
    // atan(-1.0) = -0.7853981633974483; Chrome 153 agrees
    ("atan", &[0xbff0000000000000], 0xbfe921fb54442d18),
    // atan2(1.0, 10.0) = 0.09966865249116202; fdlibm differs, Chrome 153 agrees
    ("atan2", &[0x3ff0000000000000, 0x4024000000000000], 0x3fb983e282e2cc4c),
    // atan2(1.0, 0.1) = 1.4711276743037345; fdlibm differs, Chrome 153 agrees
    ("atan2", &[0x3ff0000000000000, 0x3fb999999999999a], 0x3ff789bd2c160053),
    // atan2(1.0, 1.0) = 0.7853981633974483; Chrome 153 agrees
    ("atan2", &[0x3ff0000000000000, 0x3ff0000000000000], 0x3fe921fb54442d18),
    // atan2(1.0, -1.0) = 2.356194490192345; Chrome 153 agrees
    ("atan2", &[0x3ff0000000000000, 0xbff0000000000000], 0x4002d97c7f3321d2),
    // sinh(2.0) = 3.6268604078470186; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("sinh", &[0x4000000000000000], 0x400d03cf63b6e19f),
    // sinh(-2.0) = -3.6268604078470186; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("sinh", &[0xc000000000000000], 0xc00d03cf63b6e19f),
    // sinh(3.141592653589793) = 11.548739357257746; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("sinh", &[0x400921fb54442d18], 0x402718f45d72e671),
    // sinh(0.7) = 0.7585837018395335; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("sinh", &[0x3fe6666666666666], 0x3fe8465153d5bdbd),
    // sinh(1.0) = 1.1752011936438014; Chrome 153 agrees
    ("sinh", &[0x3ff0000000000000], 0x3ff2cd9fc44eb982),
    // sinh(-1.0) = -1.1752011936438014; Chrome 153 agrees
    ("sinh", &[0xbff0000000000000], 0xbff2cd9fc44eb982),
    // cosh(10.0) = 11013.232920103323; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("cosh", &[0x4024000000000000], 0x40c5829dd053712d),
    // cosh(2.718281828459045) = 7.610125138662287; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("cosh", &[0x4005bf0a8b145769], 0x401e70c4a4f41684),
    // cosh(1.0) = 1.5430806348152437; fdlibm differs, Chrome 153 agrees
    ("cosh", &[0x3ff0000000000000], 0x3ff8b07551d9f550),
    // cosh(-1.0) = 1.5430806348152437; fdlibm differs, Chrome 153 agrees
    ("cosh", &[0xbff0000000000000], 0x3ff8b07551d9f550),
    // cosh(0.5) = 1.1276259652063807; Chrome 153 agrees
    ("cosh", &[0x3fe0000000000000], 0x3ff20ac1862ae8d0),
    // cosh(-0.5) = 1.1276259652063807; Chrome 153 agrees
    ("cosh", &[0xbfe0000000000000], 0x3ff20ac1862ae8d0),
    // tanh(0.7) = 0.6043677771171635; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("tanh", &[0x3fe6666666666666], 0x3fe356fb17af2e91),
    // tanh(0.99) = 0.7573623242165263; glibc 2.39 differs, Chrome 153 differs
    ("tanh", &[0x3fefae147ae147ae], 0x3fe83c4fe9b770e1),
    // tanh(-0.45533377191068425) = -0.42627348688129457; fdlibm differs, Chrome 153 agrees
    ("tanh", &[0xbfdd243042c7b9c0], 0xbfdb48109753a6a1),
    // tanh(-0.2905297850836561) = -0.2826223539140064; fdlibm differs, Chrome 153 agrees
    ("tanh", &[0xbfd2980a3d5caf80], 0xbfd2167c11cb77ef),
    // tanh(1.0) = 0.7615941559557649; Chrome 153 agrees
    ("tanh", &[0x3ff0000000000000], 0x3fe85efab514f394),
    // tanh(-1.0) = -0.7615941559557649; Chrome 153 agrees
    ("tanh", &[0xbff0000000000000], 0xbfe85efab514f394),
    // asinh(1e+300) = 691.4686750787737; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("asinh", &[0x7e37e43c8800759c], 0x40859bbfd8b83e44),
    // asinh(1.7976931348623157e+308) = 710.475860073944; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("asinh", &[0x7fefffffffffffff], 0x408633ce8fb9f87e),
    // asinh(0.7853981633974483) = 0.7212254887267797; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("asinh", &[0x3fe921fb54442d18], 0x3fe7144779e3f0ba),
    // asinh(0.1) = 0.09983407889920756; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("asinh", &[0x3fb999999999999a], 0x3fb98eb9e7e5fc3e),
    // asinh(1.0) = 0.881373587019543; Chrome 153 agrees
    ("asinh", &[0x3ff0000000000000], 0x3fec34366179d427),
    // asinh(-1.0) = -0.881373587019543; Chrome 153 agrees
    ("asinh", &[0xbff0000000000000], 0xbfec34366179d427),
    // acosh(2.0) = 1.3169578969248168; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("acosh", &[0x4000000000000000], 0x3ff5124271980435),
    // acosh(100.0) = 5.298292365610485; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("acosh", &[0x4059000000000000], 0x401531738dcbc627),
    // acosh(3.0) = 1.762747174039086; fdlibm differs, Chrome 153 agrees
    ("acosh", &[0x4008000000000000], 0x3ffc34366179d427),
    // acosh(1e+300) = 691.4686750787737; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("acosh", &[0x7e37e43c8800759c], 0x40859bbfd8b83e44),
    // acosh(10.0) = 2.993222846126381; Chrome 153 agrees
    ("acosh", &[0x4024000000000000], 0x4007f21ed1ce05d4),
    // acosh(3.141592653589793) = 1.811526272460853; Chrome 153 agrees
    ("acosh", &[0x400921fb54442d18], 0x3ffcfc02f90106c1),
    // atanh(0.5) = 0.5493061443340549; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("atanh", &[0x3fe0000000000000], 0x3fe193ea7aad030b),
    // atanh(-0.5) = -0.5493061443340549; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("atanh", &[0xbfe0000000000000], 0xbfe193ea7aad030b),
    // atanh(0.4898915903710277) = 0.5359176835616454; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("atanh", &[0x3fdf5a6241cea7d0], 0x3fe1263cd787d9db),
    // atanh(0.7332313605602581) = 0.9356805099607092; glibc 2.39 differs, fdlibm differs, Chrome 153 differs
    ("atanh", &[0x3fe776a19d4040a8], 0x3fedf11840b925ab),
    // atanh(1e-10) = 1e-10; Chrome 153 agrees
    ("atanh", &[0x3ddb7cdfd9d7bdbb], 0x3ddb7cdfd9d7bdbb),
    // atanh(1e-300) = 1e-300; Chrome 153 agrees
    ("atanh", &[0x01a56e1fc2f8f359], 0x01a56e1fc2f8f359),
    // exp(1.0) = 2.718281828459045; fdlibm differs, Chrome 153 agrees
    ("exp", &[0x3ff0000000000000], 0x4005bf0a8b145769),
    // exp(0.99) = 2.691234472349262; fdlibm differs, Chrome 153 agrees
    ("exp", &[0x3fefae147ae147ae], 0x400587a5f064dcf9),
    // exp(-1.0) = 0.36787944117144233; Chrome 153 agrees
    ("exp", &[0xbff0000000000000], 0x3fd78b56362cef38),
    // exp(0.5) = 1.6487212707001282; Chrome 153 agrees
    ("exp", &[0x3fe0000000000000], 0x3ffa61298e1e069c),
    // expm1(1.0) = 1.7182818284590453; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("expm1", &[0x3ff0000000000000], 0x3ffb7e151628aed3),
    // expm1(226.48542478244406) = 2.2981068355864857e+98; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("expm1", &[0x406c4f88998da87c], 0x545ae5c151f720ed),
    // expm1(155.9706426466649) = 5.459959739784865e+67; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("expm1", &[0x40637f0f812af0eb], 0x4e0033a227047b1f),
    // expm1(141.2523533967936) = 2.2136942452874122e+61; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("expm1", &[0x4061a813476e486b], 0x4cab8d3c9e62c477),
    // expm1(-1.0) = -0.6321205588285577; Chrome 153 agrees
    ("expm1", &[0xbff0000000000000], 0xbfe43a54e4e98864),
    // expm1(0.5) = 0.6487212707001282; Chrome 153 agrees
    ("expm1", &[0x3fe0000000000000], 0x3fe4c2531c3c0d38),
    // log(3.0) = 1.0986122886681098; fdlibm differs, Chrome 153 agrees
    ("log", &[0x4008000000000000], 0x3ff193ea7aad030b),
    // log(381577.51103321457) = 12.852069283468515; fdlibm differs, Chrome 153 agrees
    ("log", &[0x41174a260b4c4a7f], 0x4029b4426cd4d8cb),
    // log(0.5) = -0.6931471805599453; Chrome 153 agrees
    ("log", &[0x3fe0000000000000], 0xbfe62e42fefa39ef),
    // log(2.0) = 0.6931471805599453; Chrome 153 agrees
    ("log", &[0x4000000000000000], 0x3fe62e42fefa39ef),
    // log1p(2.0) = 1.0986122886681098; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("log1p", &[0x4000000000000000], 0x3ff193ea7aad030b),
    // log1p(43039.97622253359) = 10.669907876328756; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("log1p", &[0x40e503ff3d3709ec], 0x402556fe2a485463),
    // log1p(47552.706396739006) = 10.769615012290982; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("log1p", &[0x40e738169acd5581], 0x40258a0afa9899cf),
    // log1p(613901.5005969411) = 13.327591400704149; glibc 2.39 differs, fdlibm differs, Chrome 153 agrees
    ("log1p", &[0x4122bc1b004e3e05], 0x402aa7ba0f60f347),
    // log1p(1.0) = 0.6931471805599453; Chrome 153 agrees
    ("log1p", &[0x3ff0000000000000], 0x3fe62e42fefa39ef),
    // log1p(0.5) = 0.4054651081081644; Chrome 153 agrees
    ("log1p", &[0x3fe0000000000000], 0x3fd9f323ecbf984c),
    // log2(0.5) = -1.0; Chrome 153 agrees
    ("log2", &[0x3fe0000000000000], 0xbff0000000000000),
    // log2(2.0) = 1.0; Chrome 153 agrees
    ("log2", &[0x4000000000000000], 0x3ff0000000000000),
    // log10(3.141592653589793) = 0.49714987269413385; glibc 2.39 differs, Chrome 153 agrees
    ("log10", &[0x400921fb54442d18], 0x3fdfd14db31ba3bb),
    // log10(611941.6217333465) = 5.786709993112281; glibc 2.39 differs, Chrome 153 agrees
    ("log10", &[0x4122accb3e53d54c], 0x401725974def6a1f),
    // log10(366954.49352097977) = 5.564612210197296; fdlibm differs, Chrome 153 agrees
    ("log10", &[0x411665a9f95d9050], 0x40164229b406e0f0),
    // log10(0.5) = -0.3010299956639812; Chrome 153 agrees
    ("log10", &[0x3fe0000000000000], 0xbfd34413509f79ff),
    // log10(2.0) = 0.3010299956639812; Chrome 153 agrees
    ("log10", &[0x4000000000000000], 0x3fd34413509f79ff),
    // cbrt(1.0) = 1.0; Chrome 153 agrees
    ("cbrt", &[0x3ff0000000000000], 0x3ff0000000000000),
    // cbrt(-1.0) = -1.0; Chrome 153 agrees
    ("cbrt", &[0xbff0000000000000], 0xbff0000000000000),
    // pow(0.5, -0.5) = 1.4142135623730951; fdlibm differs, Chrome 153 agrees
    ("pow", &[0x3fe0000000000000, 0xbfe0000000000000], 0x3ff6a09e667f3bcd),
    // pow(0.5, 0.7853981633974483) = 0.5801918103717244; fdlibm differs, Chrome 153 agrees
    ("pow", &[0x3fe0000000000000, 0x3fe921fb54442d18], 0x3fe290ee6a5e83cf),
    // pow(1.0, 1.0) = 1.0; Chrome 153 agrees
    ("pow", &[0x3ff0000000000000, 0x3ff0000000000000], 0x3ff0000000000000),
    // pow(1.0, -1.0) = 1.0; Chrome 153 agrees
    ("pow", &[0x3ff0000000000000, 0xbff0000000000000], 0x3ff0000000000000),
    // hypot(0.7853981633974483, 0.99) = 1.2637049794426247; glibc 2.39 differs, Chrome 153 differs
    ("hypot", &[0x3fe921fb54442d18, 0x3fefae147ae147ae], 0x3ff43822b667f98d),
    // hypot(0.99, 0.7853981633974483) = 1.2637049794426247; glibc 2.39 differs, Chrome 153 differs
    ("hypot", &[0x3fefae147ae147ae, 0x3fe921fb54442d18], 0x3ff43822b667f98d),
    // hypot(1.0, 2.718281828459045) = 2.8963867315900083; fdlibm differs, Chrome 153 agrees
    ("hypot", &[0x3ff0000000000000, 0x4005bf0a8b145769], 0x40072bccce85fab5),
    // hypot(-1.0, 2.718281828459045) = 2.8963867315900083; fdlibm differs, Chrome 153 agrees
    ("hypot", &[0xbff0000000000000, 0x4005bf0a8b145769], 0x40072bccce85fab5),
    // hypot(1.0, 1.0) = 1.4142135623730951; Chrome 153 agrees
    ("hypot", &[0x3ff0000000000000, 0x3ff0000000000000], 0x3ff6a09e667f3bcd),
    // hypot(1.0, -1.0) = 1.4142135623730951; Chrome 153 agrees
    ("hypot", &[0x3ff0000000000000, 0xbff0000000000000], 0x3ff6a09e667f3bcd),
];

/// `(expression, expected)`: expected is a JS expression whose value is
/// required exactly, compared with `Object.is` (so -0 and NaN count). Most
/// are ECMA-262's; the rest (`Math.pow(2, 10)`, `Math.log10(1000)`,
/// `Math.cbrt(-27)`, `Math.hypot(3, 4)`...) have an exact result, which a
/// correctly rounded function must return.
#[rustfmt::skip]
const SPEC: &[(&str, &str)] = &[
    // Number::exponentiate (§6.1.6.1.3), for Math.pow and `**`; C's pow differs
    // on the first three.
    ("Math.pow(1, NaN)", "NaN"), ("Math.pow(1, Infinity)", "NaN"),
    ("Math.pow(-1, -Infinity)", "NaN"), ("1 ** NaN", "NaN"), ("(-1) ** Infinity", "NaN"),
    ("Math.pow(NaN, 0)", "1"), ("Math.pow(NaN, -0)", "1"), ("Math.pow(NaN, 1)", "NaN"),
    ("Math.pow(Infinity, 0.5)", "Infinity"), ("Math.pow(Infinity, -0.5)", "0"),
    ("Math.pow(-Infinity, 3)", "-Infinity"), ("Math.pow(-Infinity, 2)", "Infinity"),
    ("Math.pow(-Infinity, -3)", "-0"), ("Math.pow(-Infinity, -2)", "0"),
    ("Math.pow(0, 3)", "0"), ("Math.pow(0, -3)", "Infinity"),
    ("Math.pow(-0, 3)", "-0"), ("Math.pow(-0, 2)", "0"),
    ("Math.pow(-0, -3)", "-Infinity"), ("Math.pow(-0, -2)", "Infinity"),
    ("Math.pow(2, Infinity)", "Infinity"), ("Math.pow(0.5, Infinity)", "0"),
    ("Math.pow(2, -Infinity)", "0"), ("Math.pow(0.5, -Infinity)", "Infinity"),
    ("Math.pow(-8, 1 / 3)", "NaN"), ("Math.pow(2, 10)", "1024"), ("2 ** -1074", "5e-324"),
    ("Math.pow(10, 22)", "1e22"), ("(-2) ** 3", "-8"),
    // §21.3.2
    ("Math.acos(1)", "0"), ("Math.acos(1.5)", "NaN"), ("Math.acos(NaN)", "NaN"),
    ("Math.acosh(1)", "0"), ("Math.acosh(0.5)", "NaN"), ("Math.acosh(Infinity)", "Infinity"),
    ("Math.asin(-0)", "-0"), ("Math.asin(-1.5)", "NaN"),
    ("Math.asinh(-0)", "-0"), ("Math.asinh(-Infinity)", "-Infinity"),
    ("Math.atan(-0)", "-0"), ("Math.atan(Infinity)", "Math.PI / 2"), ("Math.atan(-Infinity)", "-Math.PI / 2"),
    ("Math.atanh(-0)", "-0"), ("Math.atanh(1)", "Infinity"), ("Math.atanh(-1)", "-Infinity"), ("Math.atanh(2)", "NaN"),
    ("Math.atan2(0, 0)", "0"), ("Math.atan2(-0, 0)", "-0"), ("Math.atan2(0, -0)", "Math.PI"),
    ("Math.atan2(-0, -0)", "-Math.PI"), ("Math.atan2(0, -1)", "Math.PI"), ("Math.atan2(-0, -1)", "-Math.PI"),
    ("Math.atan2(1, 0)", "Math.PI / 2"), ("Math.atan2(-1, -0)", "-Math.PI / 2"),
    ("Math.atan2(1, Infinity)", "0"), ("Math.atan2(-1, Infinity)", "-0"),
    ("Math.atan2(1, -Infinity)", "Math.PI"), ("Math.atan2(-1, -Infinity)", "-Math.PI"),
    ("Math.atan2(Infinity, 1)", "Math.PI / 2"), ("Math.atan2(Infinity, Infinity)", "Math.PI / 4"),
    ("Math.atan2(NaN, 1)", "NaN"),
    ("Math.cbrt(-0)", "-0"), ("Math.cbrt(-Infinity)", "-Infinity"), ("Math.cbrt(-27)", "-3"),
    ("Math.cos(-0)", "1"), ("Math.cos(Infinity)", "NaN"),
    ("Math.cosh(-0)", "1"), ("Math.cosh(-Infinity)", "Infinity"),
    ("Math.exp(-0)", "1"), ("Math.exp(-Infinity)", "0"), ("Math.exp(Infinity)", "Infinity"),
    ("Math.expm1(-0)", "-0"), ("Math.expm1(-Infinity)", "-1"), ("Math.expm1(Infinity)", "Infinity"),
    ("Math.hypot()", "0"), ("Math.hypot(-0)", "0"), ("Math.hypot(-0, -0)", "0"), ("Math.hypot(-3)", "3"),
    ("Math.hypot(NaN, Infinity)", "Infinity"), ("Math.hypot(-Infinity, NaN)", "Infinity"),
    ("Math.hypot(NaN, 1)", "NaN"), ("Math.hypot(3, 4)", "5"),
    ("Math.log(-0)", "-Infinity"), ("Math.log(1)", "0"), ("Math.log(-1)", "NaN"), ("Math.log(Infinity)", "Infinity"),
    ("Math.log1p(-0)", "-0"), ("Math.log1p(-1)", "-Infinity"), ("Math.log1p(-2)", "NaN"),
    ("Math.log10(0)", "-Infinity"), ("Math.log10(1)", "0"), ("Math.log10(1000)", "3"), ("Math.log10(-1)", "NaN"),
    ("Math.log2(-0)", "-Infinity"), ("Math.log2(1)", "0"), ("Math.log2(8)", "3"), ("Math.log2(-1)", "NaN"),
    ("Math.sin(-0)", "-0"), ("Math.sin(-Infinity)", "NaN"),
    ("Math.sinh(-0)", "-0"), ("Math.sinh(-Infinity)", "-Infinity"),
    ("Math.tan(-0)", "-0"), ("Math.tan(Infinity)", "NaN"),
    ("Math.tanh(-0)", "-0"), ("Math.tanh(Infinity)", "1"), ("Math.tanh(-Infinity)", "-1"),
];

fn sandbox() -> Sandbox {
    Sandbox::new(Limits::default()).unwrap()
}

/// Evaluates `Math.<f>` (or `expr` with `a` and `b` bound) on each case's
/// arguments and returns the result bits, as hex strings.
fn results(sb: &mut Sandbox, call: &str) -> Vec<String> {
    let cases: Vec<String> = CORRECTLY_ROUNDED
        .iter()
        .map(|(f, args, _)| {
            let args: Vec<String> = args.iter().map(|a| format!("\"{a:016x}\"")).collect();
            format!("[\"{f}\", [{}]]", args.join(","))
        })
        .collect();
    let script = format!(
        r#"
        const dv = new DataView(new ArrayBuffer(8));
        const num = h => (dv.setBigUint64(0, BigInt('0x' + h)), dv.getFloat64(0));
        const hex = x => (dv.setFloat64(0, x), dv.getBigUint64(0).toString(16).padStart(16, '0'));
        const call = {call};
        JSON.stringify([{}].map(([f, args]) => hex(call(f, ...args.map(num)))));
        "#,
        cases.join(",")
    );
    let json: String = sb.eval(&script).unwrap();
    json.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim_matches('"').to_owned())
        .collect()
}

fn check(call: &str, only_pow: bool) {
    let got = results(&mut sandbox(), call);
    assert_eq!(got.len(), CORRECTLY_ROUNDED.len());
    let wrong: Vec<String> = CORRECTLY_ROUNDED
        .iter()
        .zip(&got)
        .filter(|((f, ..), _)| !only_pow || *f == "pow")
        .filter(|((_, _, want), got)| format!("{want:016x}") != **got)
        .map(|((f, args, want), got)| {
            let args: Vec<String> = args
                .iter()
                .map(|a| f64::from_bits(*a).to_string())
                .collect();
            format!("{f}({}) = 0x{got}, want 0x{want:016x}", args.join(", "))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} of {}:\n{}",
        wrong.len(),
        got.len(),
        wrong.join("\n")
    );
}

#[test]
fn math_is_correctly_rounded() {
    check("(f, ...args) => Math[f](...args)", false);
}

#[test]
fn the_exponent_operator_is_math_pow() {
    check("(f, a, b) => f === 'pow' ? a ** b : Math[f](a, b)", true);
}

#[test]
fn values_the_spec_requires() {
    let pairs: Vec<String> = SPEC
        .iter()
        .map(|(expr, want)| format!("[{expr:?}, {expr}, {want}]"))
        .collect();
    let script = format!(
        "JSON.stringify([{}].filter(([, got, want]) => !Object.is(got, want)).map(([e, got]) => e + ' = ' + (Object.is(got, -0) ? '-0' : String(got))))",
        pairs.join(",")
    );
    let wrong: String = sandbox().eval(&script).unwrap();
    assert_eq!(wrong, "[]");
}
