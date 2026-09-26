/** Reads a text field from submitted form data; files and missing fields read as empty. */
export function formString(data: FormData, name: string): string {
  const value = data.get(name);
  return typeof value === "string" ? value : "";
}
