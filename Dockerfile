FROM node:20-slim

ENV HOME=/home/claude
ENV LANG=C.UTF-8
ENV LC_ALL=C.UTF-8
ENV TERM=xterm-256color

RUN apt-get update && apt-get install -y \
    curl \
    git \
    ca-certificates \
    sudo \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g @anthropic-ai/claude-code

RUN useradd -m -s /bin/bash -d /home/claude claude && \
    mkdir -p /home/claude/workspace /home/claude/.claude /home/claude/.config && \
    touch /home/claude/.claude.json /home/claude/.claude.json.backup && \
    chown -R claude:claude /home/claude && \
    echo "claude ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers

USER claude
WORKDIR /home/claude/workspace

CMD ["tail", "-f", "/dev/null"]
