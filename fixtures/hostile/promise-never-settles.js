// A promise that can never settle: the job queue is empty and nothing is
// pending. The render must end (Exception) instead of waiting forever.
new Promise(() => {});
