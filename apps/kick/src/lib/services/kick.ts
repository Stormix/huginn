import type { Browser } from 'puppeteer';
import puppeteer from 'puppeteer-extra';
import StealthPlugin from 'puppeteer-extra-plugin-stealth';

puppeteer.use(StealthPlugin());
interface CheckStreamerResponse {
  success: boolean;
  isLive: boolean;
  viewers: number;
  title: string;
}

export class KickService {
  private browser: Browser | null = null;
  constructor() {}

  async init() {
    if (!this.browser) {
      this.browser = await puppeteer.launch({
        headless: true,
        args: ['--no-sandbox'],
      });
    }

    const page = await this.browser.newPage();
    return page;
  }

  async checkStreamer(streamer: string): Promise<CheckStreamerResponse> {
    // const response = await this.httpClient.get(`/check/${streamer}`);
    const page = await this.init();

    const response = await page.goto(`https://kick.com/api/v2/channels/${streamer}`, {
      waitUntil: 'networkidle2',
    });

    if (!response) {
      console.warn(`Failed to check streamer ${streamer}`);
      return {
        success: false,
        isLive: false,
        viewers: 0,
        title: '',
      };
    }

    if (!response.ok) {
      console.warn(`Failed to check streamer ${streamer}`);
      return {
        success: false,
        isLive: false,
        viewers: 0,
        title: '',
      };
    }

    const data = await response.json();

    return {
      success: true,
      isLive: data.livestream.is_live,
      viewers: data.livestream.viewer_count,
      title: data.livestream.session_title,
    };
  }
}
