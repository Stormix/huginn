FROM node:23 AS base
WORKDIR /usr/src/app

# Install Chrome dependencies and Chrome itself
RUN apt-get update \
    && apt-get install -y --no-install-recommends fonts-ipafont-gothic fonts-wqy-zenhei fonts-thai-tlwg fonts-khmeros \
    fonts-kacst fonts-freefont-ttf dbus dbus-x11 xvfb wget gnupg \
    && wget -q -O - https://dl-ssl.google.com/linux/linux_signing_key.pub | apt-key add - \
    && echo "deb [arch=amd64] http://dl.google.com/linux/chrome/deb/ stable main" >> /etc/apt/sources.list.d/google.list \
    && apt-get update \
    && apt-get install -y google-chrome-stable \
    && rm -rf /var/lib/apt/lists/*

# Set the Chrome executable path
ENV CHROME_PATH=/usr/bin/google-chrome-stable

RUN npx puppeteer browsers install chrome --install-deps

FROM base AS install
RUN mkdir -p /temp/prod

# Copy only necessary workspace files
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml turbo.json /temp/prod/
COPY apps/kick/package.json /temp/prod/apps/kick/
COPY packages /temp/prod/packages/

# Install production dependencies only
WORKDIR /temp/prod
RUN corepack enable
RUN corepack use pnpm@9
RUN pnpm install --frozen-lockfile --prod

FROM base AS release
WORKDIR /usr/src/app
COPY --from=install /temp/prod/node_modules node_modules
COPY --from=install /temp/prod/apps/kick/node_modules ./apps/kick/node_modules

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml turbo.json ./
COPY apps/kick/package.json ./apps/kick/
COPY apps/kick/src ./apps/kick/src

RUN npm install -g bun

# Run the app
EXPOSE 3000/tcp
WORKDIR /usr/src/app/apps/kick
ENTRYPOINT [ "bun", "start" ]