export type ActivityLogEntry = {
  id: number;
  timestamp: string;
  message: string;
};

export function appendActivityLogEntry(
  entries: ActivityLogEntry[],
  message: string,
  now: () => Date = () => new Date()
) {
  const lastEntry = entries.length > 0 ? entries[entries.length - 1] : null;
  return [
    ...entries,
    {
      id: (lastEntry?.id ?? 0) + 1,
      timestamp: now().toISOString(),
      message
    }
  ];
}

export function buildActivityLogText(entries: ActivityLogEntry[]) {
  return entries.map((entry) => `[${entry.timestamp}] ${entry.message}`).join("\n");
}
