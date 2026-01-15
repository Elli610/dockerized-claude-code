# Build the image
docker build -t claude-code-sandbox .

# Make script executable and move to PATH
chmod +x claude-sandbox
# sudo mv claude-sandbox /usr/local/bin/

# Export your API key (add to .bashrc/.zshrc)
export ANTHROPIC_API_KEY="your-key-here"
