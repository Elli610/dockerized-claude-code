FROM node:20-slim

# Install Claude Code globally
RUN npm install -g @anthropic-ai/claude-code

# Create a working directory
WORKDIR /workspace

# Default command
ENTRYPOINT ["claude"]
