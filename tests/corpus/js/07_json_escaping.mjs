// A string constant whose source already carries escapes (quote, backslash, newline): the analyzer
// reads it, and injection re-quotes any new value through json.dumps.
const MESSAGE = "she said \"hi\"\nand left a path C:\\tmp";
console.log(MESSAGE);
