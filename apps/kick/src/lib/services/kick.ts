/* eslint-disable @typescript-eslint/no-require-imports */
import { connect } from 'puppeteer-real-browser';
import { proxyManager } from '../proxy';
interface CheckStreamerResponse {
  success: boolean;
  isLive: boolean;
  viewers: number;
  title: string;
  chatroomId: string;
}

type PuppeteerBrowser = Awaited<ReturnType<typeof connect>>['browser'];

export class KickService {
  private browser: PuppeteerBrowser | null = null;
  private requestCounter = 0;
  private readonly MAX_REQUESTS_PER_BROWSER = 10; // Rotate browser after 10 requests
  private readonly PROXY_MANAGER = proxyManager;

  constructor() {
    // Initialize by checking and removing any failed proxies
    this.PROXY_MANAGER.removeFailedProxies().catch((error) => {
      console.error('Failed to initialize proxy checker:', error);
    });
  }

  private async closeBrowser() {
    if (this.browser) {
      try {
        await this.browser.close();
      } catch (error) {
        console.warn('Error closing browser:', error);
      }
      this.browser = null;
    }
  }

  async init() {
    // Close existing browser if we have one
    await this.closeBrowser();

    // Get a proxy and verify it works
    let proxy = this.PROXY_MANAGER.getProxy();
    let proxyWorks = await this.PROXY_MANAGER.checkProxy(proxy);
    let attempts = 0;
    const maxAttempts = 3;

    // Try up to maxAttempts times to get a working proxy
    while (!proxyWorks && attempts < maxAttempts) {
      console.log(`Proxy ${proxy.host}:${proxy.port} failed check, trying another...`);
      proxy = this.PROXY_MANAGER.getProxy((p) => p.host === proxy.host && p.port === proxy.port); // Remove failed proxy
      proxyWorks = await this.PROXY_MANAGER.checkProxy(proxy);
      attempts++;
    }

    if (!proxyWorks) {
      throw new Error('Failed to find a working proxy after multiple attempts');
    }

    console.log(`Using verified proxy ${proxy.host}:${proxy.port}`);

    const { browser } = await connect({
      headless: true,
      args: ['--no-sandbox'],
      customConfig: {},
      turnstile: true,
      disableXvfb: false,
      ignoreAllFlags: false,
      proxy: {
        host: proxy.host,
        port: proxy.port,
        username: proxy.username,
        password: proxy.password,
      },
      plugins: [require('puppeteer-extra-plugin-stealth')()],
    });

    this.browser = browser;
    this.requestCounter = 0;
    console.log('New browser instance created');

    return this.browser;
  }

  async checkStreamer(streamer: string): Promise<CheckStreamerResponse> {
    // Initialize browser if not exists or if we've hit the request limit
    if (!this.browser || this.requestCounter >= this.MAX_REQUESTS_PER_BROWSER) {
      console.log(`Rotating browser after ${this.requestCounter} requests`);
      await this.init();
    }

    // Increment request counter
    this.requestCounter++;

    // Create new page for this request
    const page = await this.browser!.newPage();
    try {
      const response = await page.goto(`https://api.ipify.org/?format=json`, {
        waitUntil: 'networkidle2',
      });

      if (!response || !response.ok()) {
        console.warn(`Failed to check streamer ${streamer}, status: ${response?.status()}`);
        return {
          success: false,
          isLive: false,
          viewers: 0,
          title: '',
          chatroomId: '',
        };
      }

      const data = await response.json();
      const text = await response.text();

      console.info('Reponse: ', text.substring(0, 100));

      return {
        success: true,
        isLive: data.livestream?.is_live ?? false,
        viewers: data.livestream?.viewer_count ?? 0,
        title: data.livestream?.session_title ?? '',
        chatroomId: data.chatroom?.id ?? '',
      };
    } catch (error) {
      console.error(`Error checking streamer ${streamer}:`, error);
      return {
        success: false,
        isLive: false,
        viewers: 0,
        title: '',
        chatroomId: '',
      };
    } finally {
      // Always close the page to free up resources
      try {
        await page.close();
      } catch (error) {
        console.warn('Error closing page:', error);
      }
    }
  }

  // Cleanup method to be called when shutting down
  async cleanup() {
    await this.closeBrowser();
  }
}
