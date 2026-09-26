export function requestMessage(error: unknown) {
  return error instanceof Error
    ? error.message
    : "The request could not be completed.";
}
