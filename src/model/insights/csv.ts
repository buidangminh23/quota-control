/**
 * RFC 4180 CSV: quoted fields may hold commas, quotes (doubled) and line breaks; CRLF and LF both
 * end a record; a leading byte-order mark is ignored. Returns objects keyed by the header row.
 */
export function parseCsv(text: string): Array<Record<string, string>> {
  const records = parseRecords(text.charCodeAt(0) === 0xfeff ? text.slice(1) : text);
  const header = records.shift();
  if (!header) return [];
  return records
    .filter((record) => record.length > 1 || (record[0] ?? "") !== "")
    .map((record) => Object.fromEntries(header.map((name, index) => [name, record[index] ?? ""])));
}

function parseRecords(text: string): string[][] {
  const records: string[][] = [];
  let record: string[] = [];
  let field = "";
  let quoted = false;
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index]!;
    if (quoted) {
      if (character === '"') {
        if (text[index + 1] === '"') {
          field += '"';
          index += 1;
        } else {
          quoted = false;
        }
      } else {
        field += character;
      }
      continue;
    }
    if (character === '"') {
      quoted = true;
    } else if (character === ",") {
      record.push(field);
      field = "";
    } else if (character === "\n" || character === "\r") {
      if (character === "\r" && text[index + 1] === "\n") index += 1;
      record.push(field);
      records.push(record);
      record = [];
      field = "";
    } else {
      field += character;
    }
  }
  if (field !== "" || record.length > 0) {
    record.push(field);
    records.push(record);
  }
  return records;
}
