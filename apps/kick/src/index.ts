import { Hono } from 'hono';
import { KickService } from './lib/services/kick';

const app = new Hono();
const service = new KickService();

app.get('/', (c) => c.json({ health: 'ok' }));

app.get('/check/:streamer', async (c) => {
  const streamer = c.req.param('streamer');
  const result = await service.checkStreamer(streamer);

  return c.json({
    isLive: result,
  });
});

export default {
  port: 3000,
  fetch: app.fetch,
};
