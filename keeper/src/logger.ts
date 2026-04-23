import pino from "pino";

export function createLogger(level: string): pino.Logger {
  return pino({
    level,
    transport:
      process.env.NODE_ENV !== "production"
        ? { target: "pino-pretty", options: { colorize: true } }
        : undefined,
  });
}

export type Logger = pino.Logger;
