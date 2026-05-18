# tripwires

- When replacing content between start/end markers, use rfind for the closing marker — find will match the first occurrence inside the content if it happens to contain the marker string, corrupting the file. _(from #5)
