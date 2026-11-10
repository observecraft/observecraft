# oc-browser: Playwright's official image (Apache-2.0) carries Node and a matching Chromium.
FROM mcr.microsoft.com/playwright:v1.65.0-noble
WORKDIR /app
ENV NODE_ENV=production PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
COPY package.json package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force
COPY src ./src
USER pwuser
EXPOSE 8095
CMD ["node", "src/main.ts"]
