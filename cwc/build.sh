#!/bin/bash

# Function to increment version number
increment_version() {
    local version=$1
    local major=$(echo $version | cut -d. -f1)
    local minor=$(echo $version | cut -d. -f2)
    local patch=$(echo $version | cut -d. -f3)
    
    # Increment patch version
    patch=$((patch + 1))
    
    echo "${major}.${minor}.${patch}"
}

# Path to manifest.json and index.html
MANIFEST_PATH="src/manifest.json"
INDEX_PATH="src/control/index.html"

# Check if files exist
if [ ! -f "$MANIFEST_PATH" ]; then
    echo "Error: manifest.json not found at $MANIFEST_PATH"
    exit 1
fi

if [ ! -f "$INDEX_PATH" ]; then
    echo "Error: index.html not found at $INDEX_PATH"
    exit 1
fi

# Get current identity version (the one under control.identity)
CURRENT_IDENTITY_VERSION=$(grep -A 10 '"identity":' "$MANIFEST_PATH" | grep '"version":' | sed 's/.*"version": *"\([^"]*\)".*/\1/')

if [ -z "$CURRENT_IDENTITY_VERSION" ]; then
    echo "Error: Could not find identity version in manifest.json"
    exit 1
fi

echo "Current identity version: $CURRENT_IDENTITY_VERSION"

# Increment identity version
NEW_IDENTITY_VERSION=$(increment_version "$CURRENT_IDENTITY_VERSION")
echo "New identity version: $NEW_IDENTITY_VERSION"

# Update identity version in manifest.json using sed
sed -i.bak "s/\"version\": *\"$CURRENT_IDENTITY_VERSION\"/\"version\": \"$NEW_IDENTITY_VERSION\"/" "$MANIFEST_PATH"

# Remove backup file
rm "${MANIFEST_PATH}.bak"

echo "Updated identity version in manifest.json from $CURRENT_IDENTITY_VERSION to $NEW_IDENTITY_VERSION"

# Get current build number from index.html
CURRENT_BUILD=$(grep 'BUILD [0-9]*' "$INDEX_PATH" | sed 's/.*BUILD \([0-9]*\).*/\1/')

if [ -z "$CURRENT_BUILD" ]; then
    echo "Warning: Could not find build number in index.html, setting to 1"
    CURRENT_BUILD=0
fi

# Increment build number
NEW_BUILD=$((CURRENT_BUILD + 1))

echo "Incrementing build number from $CURRENT_BUILD to $NEW_BUILD"

# Update build number in index.html
sed -i.bak "s/BUILD [0-9]*/BUILD $NEW_BUILD/" "$INDEX_PATH"

# Remove backup file
rm "${INDEX_PATH}.bak"

echo "Updated build number in index.html to $NEW_BUILD"

# Create zip file
ZIP_NAME="{551BF148-3F0D-4293-99C2-C9C3A1A6A073}.zip"

echo "Creating zip file: $ZIP_NAME"

# Remove existing zip if it exists
if [ -f "$ZIP_NAME" ]; then
    rm "$ZIP_NAME"
    echo "Removed existing zip file"
fi

# Create zip of src directory contents
cd src && zip -r "../$ZIP_NAME" . && cd ..

if [ $? -eq 0 ]; then
    echo "Successfully created $ZIP_NAME"
else
    echo "Error: Failed to create zip file"
    exit 1
fi

echo "Build complete!"