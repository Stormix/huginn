import { z } from 'zod';

const envSchema = z.object({
  PROXY_LIST: z.string(),
  SERVER_IPS: z.string(),
});

export const env = envSchema.parse(process.env);
