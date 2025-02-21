import axios, { AxiosError } from 'axios';
import { HttpsProxyAgent } from 'https-proxy-agent';
import { env } from './env';

interface Proxy {
  host: string;
  port: number;
  username: string;
  password: string;
}

export class ProxyManager {
  private proxyList: Proxy[] = [];
  private currentIndex = 0;
  private serverIPs: string[];

  constructor() {
    this.proxyList = env.PROXY_LIST.split('\n').map((proxy) => {
      const [host, port, username, password] = proxy.split(':');
      return { host, port: parseInt(port), username, password };
    });
    this.serverIPs = env.SERVER_IPS.split(',').map((ip) => ip.trim());
    console.log(`Loaded ${this.proxyList.length} proxies and ${this.serverIPs.length} server IPs`);
  }

  getProxy(shouldRemove?: (proxy: Proxy) => boolean): Proxy {
    if (this.proxyList.length === 0) {
      throw new Error('No proxies available');
    }

    const proxy = this.proxyList[this.currentIndex];
    this.currentIndex = (this.currentIndex + 1) % this.proxyList.length;

    // if (shouldRemove) {
    //   if (shouldRemove(proxy)) {
    //     console.log(`Removing proxy ${proxy.host}:${proxy.port}`);
    //     this.proxyList = this.proxyList.filter((p) => !shouldRemove(p));
    //   }
    // }

    console.log(`Using proxy ${proxy.host}:${proxy.port}`);

    return proxy;
  }

  async checkProxy(proxy: Proxy, maxRetries = 3): Promise<boolean> {
    const { host, port, username, password } = proxy;
    const service = 'https://api.ipify.org/?format=json';

    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        if (attempt > 0) {
          // Exponential backoff: wait 2^attempt seconds (1s, 2s, 4s)
          await new Promise((resolve) => setTimeout(resolve, Math.pow(2, attempt) * 1000));
          console.log(`Retry attempt ${attempt + 1} for proxy ${host}:${port}`);
        }

        const response = await axios.get(service, {
          httpsAgent: new HttpsProxyAgent(`http://${username}:${password}@${host}:${port}`),
          timeout: 10000, // 10 second timeout
        });

        // Extract IP from response (handle different API formats)
        const ip = response.data.ip || response.data.query;

        // Check if the IP matches any of our server IPs
        if (this.serverIPs.includes(ip)) {
          console.warn(`Proxy ${host}:${port} is using server IP ${ip}`);
          return false;
        }

        // If we got here, the proxy is working and using a different IP
        return true;
      } catch (error) {
        const axiosError = error as AxiosError;
        console.warn(
          `IP check service ${service} failed for proxy ${host}:${port} (attempt ${attempt + 1}/${maxRetries}):`,
          axiosError.message,
        );

        // If this was our last attempt, return false
        if (attempt === maxRetries - 1) {
          return false;
        }
      }
    }

    return false;
  }

  async removeFailedProxies(): Promise<void> {
    const failedProxies: Proxy[] = [];

    for (const proxy of this.proxyList) {
      const isWorking = await this.checkProxy(proxy);
      if (!isWorking) {
        failedProxies.push(proxy);
      }
    }

    if (failedProxies.length > 0) {
      this.proxyList = this.proxyList.filter(
        (proxy) => !failedProxies.some((failed) => failed.host === proxy.host && failed.port === proxy.port),
      );
      console.log(`Removed ${failedProxies.length} failed proxies. ${this.proxyList.length} proxies remaining`);
    }
  }
}

export const proxyManager = new ProxyManager();
