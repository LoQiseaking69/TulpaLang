// highlight.js grammar for Tulpa (.rav / .tlp), mirroring editors/vscode/syntaxes/tulpa.tmLanguage.json
export default function tulpa(hljs) {
  const IDENT = '[A-Za-z_][A-Za-z0-9_]*';
  const KEYWORDS = {
    keyword: [
      'fn', 'def', 'let', 'const', 'mut', 'struct', 'machine', 'state', 'on', 'enter', 'exit',
      'if', 'elif', 'else', 'while', 'for', 'in', 'loop', 'break', 'continue', 'return', 'match',
      'try', 'catch', 'spawn', 'await', 'use', 'import', 'from', 'as', 'after', 'within',
      'and', 'or', 'not'
    ],
    built_in: ['print', 'len', 'signal', 'derive', 'check', 'round', 'sin', 'cos', 'sqrt', 'float', 'int', 'str', 'abs', 'min', 'max', 'range'],
    literal: ['true', 'false', 'nil', 'self', 'PI', 'TAU', 'E', 'HIGH', 'LOW', 'ARGS']
  };
  const SPEC = { className: 'meta', begin: /\b(require|ensure|invariant|assert|always|when)\b/ };
  const QUANTITY = {
    className: 'number',
    begin: /\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?(?=\[)/,
    starts: { className: 'unit', begin: /\[/, end: /\]/, excludeBegin: false, excludeEnd: false }
  };
  const UNIT_TYPE = { className: 'unit', begin: /(?<=(?::|->)\s*)\[[A-Za-z][A-Za-z0-9_*/^. -]*\]/ };
  const NUMBER = {
    className: 'number',
    variants: [
      { begin: /\b0[xX][0-9a-fA-F_]+\b/ },
      { begin: /\b0[bB][01_]+\b/ },
      { begin: /\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?\b/ }
    ]
  };
  const SUBST = { className: 'subst', begin: /\{/, end: /\}/, keywords: KEYWORDS };
  const STRING = {
    className: 'string',
    variants: [
      { begin: /f"/, end: /"/, contains: [hljs.BACKSLASH_ESCAPE, SUBST] },
      { begin: /"/, end: /"/, contains: [hljs.BACKSLASH_ESCAPE] },
      { begin: /'/, end: /'/, contains: [hljs.BACKSLASH_ESCAPE] }
    ]
  };
  SUBST.contains = [STRING, QUANTITY, NUMBER];
  return {
    name: 'Tulpa',
    aliases: ['rav', 'tlp'],
    keywords: KEYWORDS,
    contains: [
      hljs.HASH_COMMENT_MODE,
      hljs.C_LINE_COMMENT_MODE,
      hljs.C_BLOCK_COMMENT_MODE,
      STRING,
      SPEC,
      { className: 'title.function', begin: new RegExp('(?<=\\b(fn|def)\\s+)' + IDENT) },
      { className: 'title.class', begin: new RegExp('(?<=\\b(struct|machine|state)\\s+)' + IDENT) },
      { className: 'title.class', begin: /(?<==>\s*)[A-Z][A-Za-z0-9_]*/ },
      { className: 'built_in', begin: /\b(GPIO|I2C|SPI|UART|PWM|Servo|PID|CAN|BLE)\b/ },
      { className: 'built_in', begin: /\b[a-z_]+(?=::)/ },
      { className: 'macro', begin: /\b[A-Za-z_][A-Za-z0-9_]*!/ },
      { className: 'variable', begin: /\b[A-Za-z_][A-Za-z0-9_]*'/ },
      UNIT_TYPE,
      QUANTITY,
      NUMBER,
      { className: 'operator', begin: /=>|\|>|\*\*|->/ }
    ]
  };
}
