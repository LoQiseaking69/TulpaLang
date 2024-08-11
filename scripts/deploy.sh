#!/bin/bash
# Deployment script for Tulpa

set -e  # Exit immediately if a command exits with a non-zero status

echo "Starting Tulpa deployment process..."

# Define the deployment directory (update this path as needed)
DEPLOYMENT_DIR="/path/to/deployment/directory"

# Check if the deployment directory exists
if [ ! -d "$DEPLOYMENT_DIR" ]; then
    echo "Error: Deployment directory does not exist. Creating it now..."
    mkdir -p "$DEPLOYMENT_DIR"
    echo "Deployment directory created."
fi

# Step 1: Clean the deployment directory
echo "Cleaning the deployment directory..."
rm -rf "$DEPLOYMENT_DIR"/*
echo "Deployment directory cleaned."

# Step 2: Copy the release build to the deployment directory
echo "Copying release build to deployment directory..."
cp -r target/release/* "$DEPLOYMENT_DIR/"
echo "Release build copied."

# Step 3: Set executable permissions
echo "Setting executable permissions on binaries..."
find "$DEPLOYMENT_DIR" -type f -name "tulpa" -exec chmod +x {} \;
echo "Permissions set."

# Step 4: Optional - Restart the application or service
# Uncomment and modify the following lines if you need to restart a service after deployment
# echo "Restarting the Tulpa service..."
# sudo systemctl restart tulpa.service
# echo "Tulpa service restarted."

# Final message
echo "Tulpa deployment process completed successfully."
